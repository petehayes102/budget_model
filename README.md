# Budget Model

An API for modelling financial transactions.

## Development

### Enabling logging in test pack

Add this line to the beginning of each test you want to enable logging for:

```rust
let _ = env_logger::builder().is_test(true).try_init();
```
