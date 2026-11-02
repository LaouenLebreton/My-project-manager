# The Super PM team server (crates/pm-server) as a small container.
#
#   docker build -t super-pm-server .
#   docker run -p 8080:8080 -v super-pm-data:/data \
#     -e SUPER_PM_PUBLIC_URL=https://pm.example.com super-pm-server
#
# It serves plain HTTP on port 8080; put HTTPS in front of it (the hosting provider's, or Caddy).
# Everything it stores is in /data: keep that on a volume and back it up. See docs/deploy.md.

FROM rust:1.99-slim-trixie AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p pm-server --bin super-pm-server

FROM debian:trixie-slim
RUN useradd --system --home /data super-pm && mkdir /data && chown super-pm /data
COPY --from=build /src/target/release/super-pm-server /usr/local/bin/super-pm-server
USER super-pm
ENV SUPER_PM_DATA=/data \
    SUPER_PM_LISTEN=0.0.0.0:8080
VOLUME /data
EXPOSE 8080
CMD ["super-pm-server"]
