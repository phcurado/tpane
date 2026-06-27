# Install

## Install script

```sh
curl -fsSL https://raw.githubusercontent.com/phcurado/tpane/main/install.sh | sh
```

## crates.io

```sh
cargo install tpane
```

## mise

```sh
mise use -g github:phcurado/tpane@latest
```

Update with:

```sh
mise upgrade github:phcurado/tpane
```

## Source

From the repo root:

```sh
cargo install --path . --locked --force
```

Check the installed version:

```sh
tpane --version
```
