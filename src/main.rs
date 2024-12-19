// This program downloads secrets from AWS Secrets Manager and uploads them to Kubernetes Secrets.
//
// The program is intended to be run as a Kubernetes CronJob.

use std::collections::HashMap;

use aws_config::BehaviorVersion;
use aws_sdk_secretsmanager::types::{
    builders::FilterBuilder, Filter, FilterNameStringType, SecretListEntry,
};
use base64::engine::general_purpose;
use base64::Engine;
use clap::{arg, command, Parser};
use k8s_openapi::api::core::v1::Secret;
use kube::api::{Api, Patch, PatchParams};
use log::{debug, info};

/// CLAP parser for command line arguments
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// The key of the tag for the namespace in AWS Secrets Manager
    #[arg(short, long)]
    namespace_tag: String,

    /// The key of the tag for the secret name in AWS Secrets Manager
    #[arg(short, long)]
    secret_name_tag: String,

    /// The filename key in the AWS secret
    #[arg(short, long)]
    filename_tag: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::builder().format_timestamp(None).init();

    let args = Args::parse();

    // set credentials for AWS
    let config = aws_config::load_defaults(BehaviorVersion::v2024_03_28()).await;
    let client = aws_sdk_secretsmanager::Client::new(&config);

    // get secrets that have a tag with key `namespace_tag`
    // filter by secrets with tags that have the key `namespace_tag`
    let namespace = args.namespace_tag.clone();
    let filter = Filter::builder()
        .key(FilterNameStringType::TagKey)
        .values(namespace)
        .build();
    let secrets = client
        .list_secrets()
        .filters(filter)
        .send()
        .await?
        .secret_list
        .unwrap_or_default();
    debug!("Number of secrets retrieved: {}", secrets.len());

    // for each secret, get the secret value and upload it to Kubernetes
    // the secret name in Kubernetes is the value of the tag with key `/fhm/k8s/secret-name`
    // the namespace in Kubernetes is the value of the tag with key `/fhm/k8s/namespace`
    for secret in secrets {
        info!("AWS Secret Name: {}", secret.name.clone().unwrap(),);
        let secret_name = get_name_from_aws_secret(&secret, &args.secret_name_tag);
        let namespaces = get_namespaces_from_aws_secret(&secret, &args.namespace_tag);

        let aws_key = secret.name.as_ref().unwrap();
        let secret_value = client.get_secret_value().secret_id(aws_key).send().await?;
        let secret_value: HashMap<String, String> =
            serde_json::from_str(&secret_value.secret_string.unwrap()).unwrap();

        // depending on whether the secret has the filename tag,
        // create a HashMap with the secret values
        let data_map = match get_filename_from_aws_secret(&secret, &args.filename_tag) {
            Some(filename) => create_filesecret_from_aws_secret(secret_value, filename),
            None => create_datamap_from_aws_secret(secret_value),
        };

        let client = kube::Client::try_default().await?;
        for namespace in namespaces {
            // upload_secret(&client, &namespace, &secret_name, data_map.clone()).await?;
            let secrets: Api<Secret> = Api::namespaced(client.clone(), &namespace);
            let patch = serde_json::json!({
                "apiVersion": "v1",
                "kind": "Secret",
                "metadata": {
                    "name": secret_name,
                    "namespace": namespace
                },
                "data": data_map
            });
            debug!("patch: {}", patch);

            // apply the patch
            let params = PatchParams::apply("myapp");
            let patch = Patch::Apply(&patch);
            let result = secrets.patch(&secret_name, &params, &patch).await;
            match result {
                Ok(_) => info!("Secret {}/{} updated", namespace, secret_name),
                Err(e) => info!("Error updating secret: {}", e),
            }
        }
    }
    Ok(())
}

// gets the value of the tag with key `secret_name_tag` from the AWS secret
fn get_name_from_aws_secret(secret: &SecretListEntry, secret_name_tag: &str) -> String {
    let value = secret
        .tags
        .as_ref()
        .unwrap()
        .iter()
        .find(|tag| tag.key.as_ref().unwrap() == secret_name_tag)
        .unwrap()
        .value
        .as_ref()
        .unwrap();
    String::from(value)
}

// gets the value of the tag with key `namespace_tag` from the AWS secret
fn get_namespaces_from_aws_secret(secret: &SecretListEntry, namespace_tag: &str) -> Vec<String> {
    let value = secret
        .tags
        .as_ref()
        .unwrap()
        .iter()
        .find(|tag| tag.key.as_ref().unwrap() == namespace_tag)
        .unwrap()
        .value
        .as_ref()
        .unwrap();
    value.split(' ').map(|s| String::from(s)).collect()
}

// gets the value of the tag with key `filename_tag` from the AWS secret
fn get_filename_from_aws_secret(secret: &SecretListEntry, filename_tag: &str) -> Option<String> {
    let value = secret
        .tags
        .as_ref()
        .unwrap()
        .iter()
        .find(|tag| tag.key.as_ref().unwrap() == filename_tag);
    match value {
        None => None,
        Some(value) => Some(String::from(value.value.as_ref().unwrap())),
    }
}

// creates a HashMap with the secret values encoded as a single value with in base64 and a key as the filename
fn create_filesecret_from_aws_secret(
    secrets: HashMap<String, String>,
    filename: String,
) -> HashMap<String, String> {
    use std::fmt::Write;

    let engine = general_purpose::STANDARD;
    let res: String = secrets.into_iter().fold(String::new(), |mut res, (k, v)| {
        write!(&mut res, "{}={}\n", k, v).unwrap();
        res
    });
    let encoded = engine.encode(res.as_bytes());

    HashMap::from([(filename, encoded)])
}

// creates a HashMap with the secret values encoded in base64
// each key and value is encoded separately
fn create_datamap_from_aws_secret(
    secret_value: HashMap<String, String>,
) -> HashMap<String, String> {
    let mut data_map = HashMap::<String, String>::new();
    let engine = general_purpose::STANDARD;
    for (key, value) in &secret_value {
        let encoded = engine.encode(value.as_bytes());
        data_map.insert(key.clone(), encoded);
    }
    data_map
}
