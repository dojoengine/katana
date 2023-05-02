# Use the official Rust image as the base image
FROM rust:latest as builder

# Set the working directory
WORKDIR /usr/src/katana

COPY . .

# Build the katana binary
RUN cargo build --release --bin katana

# Use a lightweight image for the deployment
FROM debian:buster-slim

# Install necessary dependencies for running the binary
RUN apt-get update && \
    apt-get install -y ca-certificates tzdata && \
    rm -rf /var/lib/apt/lists/*

# Set the working directory
WORKDIR /app

# Copy the katana binary from the builder stage
COPY --from=builder /usr/src/katana/target/release/katana .

# Expose any necessary ports for your application (if applicable)
EXPOSE 5050

# Set the entrypoint to run the katana binary
ENTRYPOINT ["./katana"]
