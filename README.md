# Key-Value Cache Service

A high-performance in-memory key-value cache service built with Rust, featuring concurrent access and automatic eviction.

## Overview

This service provides a simple HTTP API for storing and retrieving key-value pairs with the following features:
- Fast in-memory storage
- Concurrent access support
- Adaptive multi-tiered eviction strategy
- RESTful HTTP API
- Protection against Out-Of-Memory conditions

## API Documentation

### 1. PUT Operation
**HTTP Method:** POST  
**Endpoint:** `/put`  
**Request Body Format:**
```json
{
  "key": "string (max 256 characters)",
  "value": "string (max 256 characters)"
}
```
**Response:**  
On Success (HTTP 200):
```json
{
  "status": "OK",
  "message": "Key inserted/updated successfully."
}
```
On Failure:
```json
{
  "status": "ERROR",
  "message": "Error message"
}
```

### 2. GET Operation
**HTTP Method:** GET  
**Endpoint:** `/get`  
**Parameters:** A query parameter named `key`  
**Example URL:** `/get?key=exampleKey`  
**Response:**  
On Success (HTTP 200):
```json
{
  "status": "OK",
  "key": "exampleKey",
  "value": "the corresponding value"
}
```
If Key Not Found:
```json
{
  "status": "ERROR",
  "message": "Key not found."
}
```
On Other Failures:
```json
{
  "status": "ERROR",
  "message": "Error description"
}
```

## Building and Running the Docker Image

### Build the Docker Image

```bash
docker build -t keyvalue-cache .
```

### Run the Docker Container

```bash
docker run -p 7171:7171 keyvalue-cache
```

The service will be available at `http://localhost:7171`.

### Troubleshooting

#### GLIBC Version Issues

If you encounter errors related to missing GLIBC versions like:
```
./keyvalue-cache: /lib/x86_64-linux-gnu/libc.so.6: version 'GLIBC_2.33' not found
```

This indicates a compatibility issue between the build environment and runtime environment. The Dockerfile has been updated to use Ubuntu 22.04, which includes newer GLIBC versions required by the Rust compiler. If you're still experiencing issues:

1. Make sure you're using the latest Dockerfile from this repository
2. Rebuild the Docker image completely (not using cache):
   ```bash
   docker build --no-cache -t keyvalue-cache .
   ```
3. If issues persist, you may need to modify the Dockerfile to use an even newer base image.

## Design Choices and Optimizations

### 1. Concurrent Data Structure
- The service uses **DashMap**, a concurrent hash map implementation in Rust, which provides high-performance concurrent access without requiring explicit locking.
- This choice allows for better throughput when handling multiple concurrent requests, especially on multi-core systems.

### 2. Advanced Memory Management & Eviction Strategy
- **Multi-tiered eviction algorithm** that combines CLOCK and LRU strategies:
  - Primary: Clock algorithm for normal conditions
  - Secondary: LRU (Least Recently Used) when memory pressure is high
- **Adaptive eviction thresholds** based on current memory usage:
  - Below 70% memory usage: Maintains full cache capacity (zero cache miss guarantee)
  - Above 70% memory usage: Gradually reduces max entries based on memory pressure
  - Emergency eviction at 95% memory usage to prevent OOM situations
- **Memory monitoring** using both system metrics (on Linux) and fallback to size-based estimation
- **Dynamic eviction** that calibrates its aggressiveness based on actual system conditions
- **Zero cache miss guarantee** when memory usage is below 70%, by preserving all entries without eviction

### 3. Memory Usage Optimization
- **Access tracking** with a monotonic counter rather than timestamps to minimize overhead
- **Efficient metadata storage** with atomic values to avoid locks
- **Targeted eviction** that removes only what's necessary to maintain stability
- **Staged eviction process** that minimizes impact on performance by using increasing levels of aggressiveness
- **Memory usage monitoring** to maintain system stability


### 4. Performance Tuning for AWS t3.small (2 cores, 2GB RAM)
- Environment variables configured to optimize resource usage on a t3.small instance.
- Worker threads limited to match available CPU cores to prevent context-switching overhead.
- Memory monitoring and management calibrated for 2GB RAM capacity.
- Proactive eviction prevents page swapping which would severely impact performance.

### 5. Request/Response Format
- Standardized JSON response format for all API endpoints.
- Clear distinction between success and error states.
- Input validation to prevent oversized keys or values (>256 characters).

### 6. Async Runtime
- Built on Actix-web and Tokio, providing efficient async handling of requests.
- Background eviction task runs on a separate timer, not blocking the main request handling threads. 