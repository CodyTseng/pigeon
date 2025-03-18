# Pigeon

Pigeon is a simple command-line tool that helps you connect to a pigeon nest service to expose your local Nostr relay to the internet.

## Build

```bash
git clone https://github.com/CodyTseng/pigeon.git
cd pigeon/pigeon
cargo build --release
```

## Usage

```bash
pigeon -t ws://localhost:4869
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
