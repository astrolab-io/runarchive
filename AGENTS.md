# runarchive

`runarchive` is a high-performance, asynchronous, zero-copy ZIP parser designed natively for both standard operating systems (Linux, macOS, Windows) and WebAssembly (WASM). Support for WASI is on the roadmap.

By treating remote archives (like those hosted on HTTP servers or S3 buckets) as random-access memory constructs via intelligent `Range` requests, `runarchive` drastically reduces bandwidth and memory footprints. It entirely bypasses the traditional requirement of downloading multi-gigabyte ZIP files to disk before extraction.

## Code Style

- ...
- ...
- ...

## Architecture

- ...
- ...
- ...

## Testing

- Write unit tests for all business logic
- Maintain >80% code coverage
- ...

## Security

- Never commit API keys or secrets
- Validate all user inputs
- Use parameterized queries for database access
