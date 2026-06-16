FROM rust:1.87-slim AS builder

WORKDIR /app
COPY . .
RUN cargo build --release -p openmgmt-server

FROM debian:bookworm-slim

COPY --from=builder /app/target/release/openmgmt-server /usr/local/bin/openmgmt-server

EXPOSE 3000
CMD ["openmgmt-server"]
