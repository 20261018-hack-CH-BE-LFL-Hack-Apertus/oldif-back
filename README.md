# oldif-back

Oldif archival authenticated API and realtime research backend.

## Setup / Configuración / Einrichtung / Configuration

Copy `.env.example` to `.env`, configure the listed variables and a random internal service token. Use Rust 1.99.1 and the committed lockfile.

```sh
cargo test --locked
cargo run --locked
```

Build an independent OCI image with Apple Container:

```sh
container build --tag oldif-back:dev .
```

`docker-compose.yml` is for local development only. Production uses Terraform-managed Azure Container Apps. Local setup values belong in `.env`; production values are injected from Key Vault.
