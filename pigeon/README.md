# Pigeon

Pigeon is a simple program that helps you connect to a pigeon nest service to expose your local Nostr relay to the internet.

## Prerequisites

You need to have Rust installed on your machine. If you don't have it yet, you can install it via [rustup](https://rustup.rs/).

## Build

```bash
# Clone the repository
git clone https://github.com/CodyTseng/pigeon.git

# Change the working directory
cd pigeon

# Build the project
cargo build -p pigeon --release
```

The binary will be available at `./target/release/pigeon`.

## Usage

```bash
pigeon -t ws://localhost:4869 # Your local relay URL
```

### Command Line Arguments

| Option         | Short | Description                                                         |
| -------------- | ----- | ------------------------------------------------------------------- |
| `--relay`      | `-r`  | URL of the relay you want to expose (default: ws://localhost:4869/) |
| `--proxy`      | `-p`  | URL of the proxy service (default: wss://proxy.nostr-relay.app/)    |
| `--secret-key` | `-s`  | Optional hex or bech32 secret key for authentication                |
| `--help`       | `-h`  | Print help information                                              |
| `--version`    | `-v`  | Print version information                                           |

### Example

Expose a local relay running on port 4869:

```bash
pigeon -r ws://localhost:4869
```

Use a custom proxy service:

```bash
pigeon -r ws://localhost:4869 -p wss://your-proxy-service.com
```

Use a specific secret key:

```bash
pigeon -r ws://localhost:4869 -s nsec1...
```

## License

MIT
