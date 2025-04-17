# Use the official Rust image as a base image
FROM rust:1.86.0 AS builder

# Set the working directory
WORKDIR /usr/src/app

# Install necessary dependencies for OpenCV and Clang
RUN apt-get update && apt-get install -y \
    libopencv-dev \
    clang \
    && rm -rf /var/lib/apt/lists/*

# Copy the Cargo.toml and Cargo.lock files
COPY Cargo.toml Cargo.lock ./

# Copy the source code
COPY src ./src

# Build the project
RUN cargo build --release

# Use a minimal base image for the final stage
FROM debian:stable-slim

# Set the working directory
WORKDIR /usr/src/app

# Install runtime dependencies for OpenCV
RUN apt-get update && apt-get install -y \
    libopencv-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy the built binary from the builder stage
COPY --from=builder /usr/src/app/target/release/server .

# Expose the port the app runs on
EXPOSE 8000

# Command to run the application
CMD ["./server"]