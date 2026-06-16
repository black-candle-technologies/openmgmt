FROM rust:1.96-slim AS builder

WORKDIR /app
COPY . .
RUN cargo build --locked --release -p openmgmt-server

FROM gcr.io/distroless/cc-debian12

COPY --from=builder /app/target/release/openmgmt-server /usr/local/bin/openmgmt-server

EXPOSE 3000
CMD ["/usr/local/bin/openmgmt-server"]
