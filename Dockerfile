FROM --platform=$BUILDPLATFORM rust:1.88-slim-bullseye AS builder
ARG TARGETARCH
RUN apt-get update && apt-get install -y \
    build-essential musl-tools gcc-aarch64-linux-gnu &&\
    rm -rf /var/lib/apt/lists/*

ENV RUST_TARGET_amd64=x86_64-unknown-linux-musl
ENV RUST_TARGET_arm64=aarch64-unknown-linux-musl
RUN rustup target add $(eval echo \$RUST_TARGET_${TARGETARCH})

WORKDIR /app
COPY . .

ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-gnu-gcc

RUN TARGET=$(eval echo \$RUST_TARGET_${TARGETARCH}) && \
    cargo build --release --locked --target $TARGET && \
    cp target/$TARGET/release/glance-github-graph glance-github-graph

FROM scratch
WORKDIR /app
COPY --from=builder /app/templates ./templates
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/

EXPOSE 8080
COPY --from=builder /app/glance-github-graph ./glance-github-graph
CMD ["./glance-github-graph"] 
