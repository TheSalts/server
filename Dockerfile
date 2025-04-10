# Use the official Rust image as a base image
FROM rust:1.70 as builder

# Set the working directory
WORKDIR /app

# Copy the Cargo.toml and Cargo.lock files
COPY Cargo.toml Cargo.lock ./

# Copy the source code
COPY src ./src

# Build the application
RUN cargo build --release

# Use a minimal base image for the final stage
FROM debian:buster-slim

# Set the working directory
WORKDIR /app

# Copy the built binary from the builder stage
COPY --from=builder /app/target/release/server ./server

# Expose the port the server will run on
EXPOSE 8000

# Command to run the application
CMD ["./server"]