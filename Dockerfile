FROM rust:1.59.0 AS builder

RUN USER=root cargo new --bin shut2
WORKDIR /shut2

# Cache dependencies
COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml

RUN cargo build --bin shut2 --release
RUN rm src/*.rs
RUN rm /shut2/target/release/deps/shut2*

# Build App
COPY ./src ./src
RUN cargo build --bin shut2 --release

# Final image
FROM debian:buster-slim
WORKDIR /usr/app/

# Copy the executable
RUN apt-get update && apt-get install -y libssl1.1 ca-certificates sqlite3 && rm -rf /var/lib/apt/lists/*
COPY --from=builder /shut2/target/release/shut2 /usr/app/

# Start command
CMD [ "./shut2" ]