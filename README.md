# FOUR.MEME Sandwich Bot

This bot is built by [@tonyke-bot](https://github.com/tonyke-bot) and [@shouc](https://github.com/shouc) using revm and artemis. 

### Build
Ensure you have Rust installed via https://rustup.rs/
```
cargo build --release
```

### Run
Backtest on a tx (e.g., [0x55743...f8ec2](https://bscscan.com/tx/0x557430d9a09e6ea985fada2588152225bffab5c0ef0d53e5a9c666804baf8ec2)):
```
cargo run --release -- run --ipc-url [Your BSC Node IPC] 0x557430d9a09e6ea985fada2588152225bffab5c0ef0d53e5a9c666804baf8ec2
```

Start the bot:

```
cargo run --release -- start --ipc-url [Your BSC Node IPC] --private-key [Your Private Key with >8BNB]
```
