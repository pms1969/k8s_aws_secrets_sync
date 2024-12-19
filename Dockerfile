FROM rust:1.82 AS builder

# ! needed for rustls to compile for the x86_64-unknown-linux-musl target.
RUN apt update \
    && apt install -y clang musl-tools

WORKDIR /usr/src/k8s_aws_secrets_sync
COPY Cargo.toml Cargo.lock ./

# creates dummy main to compile dependencies against.
# this prevents local builds from having to build everything in the event
# there are no changes to the dependencies.
RUN mkdir src
# && echo "fn main() {print!(\"Dummy Uploader\");}" > src/main.rs

# ! needed to target `scratch` image
RUN rustup target install x86_64-unknown-linux-musl

# build dependencies
# RUN cargo build --release --target x86_64-unknown-linux-musl

# build Uploader
COPY src ./src
RUN cargo install --target x86_64-unknown-linux-musl --path .

# * Create the release image.
FROM scratch
COPY --from=builder /usr/local/cargo/bin/k8s_aws_secrets_sync /usr/local/bin/k8s_aws_secrets_sync
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
USER 1709
ENTRYPOINT ["k8s_aws_secrets_sync"]
CMD ["--help"]
