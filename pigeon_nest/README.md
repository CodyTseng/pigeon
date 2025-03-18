# Pigeon Nest

A simple Nostr relay proxy service

## How to Start the Service (Development)

1. Install Rust (via rustup if not already installed).
2. Copy the `.env.example` file to `.env` and adjust configurations if needed:
   ```bash
   cp .env.example .env
   ```
3. Run the service:
   ```bash
   cargo run
   ```

## How to Deploy the Service

1. Build the project for production:
   ```bash
   cargo build --release
   ```
2. Copy the executable from `target/release/` to your production environment.
3. Create a systemd service unit file (e.g., `/etc/systemd/system/pigeon-nest.service`):

   ```ini
   [Unit]
   Description=Pigeon Nest Service
   After=network.target

   [Service]
   ExecStart=/path/to/pigeon-nest
   Restart=always
   RestartSec=5
   User=YOUR_USER_NAME
   Environment=RUST_LOG=info
   Environment=PORT=3000
   # The actual endpoint you expose to the public
   Environment=ENDPOINT=ws://localhost:3000

   [Install]
   WantedBy=multi-user.target
   ```

4. Enable and start the service:
   ```bash
   sudo systemctl enable pigeon-nest
   sudo systemctl start pigeon-nest
   ```
5. Verify the service status:
   ```bash
   sudo systemctl status pigeon-nest
   ```

## License

MIT
