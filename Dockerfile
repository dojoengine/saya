# Consider compiling the cairo programs before building the image to embed them.

FROM rust:alpine AS build

RUN apk add --update alpine-sdk linux-headers libressl-dev

WORKDIR /src
COPY ./rust-toolchain.toml /src/

# Cache Docker layer for nightly toolchain installation
RUN cargo --version

COPY . /src/

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    cargo build --release

RUN --mount=type=cache,target=/src/target \
    mkdir ./build && \
    cp ./target/release/saya ./build/

FROM alpine

LABEL org.opencontainers.image.source=https://github.com/dojoengine/saya

COPY --from=build /src/build/saya /usr/bin/
COPY programs /programs

ENTRYPOINT [ "saya" ]
