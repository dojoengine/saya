# Consider building the cairo programs before building the image to embed them.

FROM rust:alpine AS build

RUN apk add --update alpine-sdk linux-headers libressl-dev

COPY . /src
WORKDIR /src

RUN cargo build --release

FROM alpine

LABEL org.opencontainers.image.source=https://github.com/dojoengine/saya

COPY --from=build /src/target/release/saya /usr/bin/
COPY programs /programs

ENTRYPOINT [ "saya" ]
