# Coinline CLI

Coinline is a command line interface (CLI) to interact with Bitcoin hardware wallets, the Bitcoin network, and broadcast transactions without a need to run a Bitcoin node. Coinline is currently compatible with **********************************************Coldcard MK4, Keystone, and Ledger Nano S.********************************************** This is a wallet designed for technical users. For more information, visit https://coinline.dev

## Quick Start

`npm install -g coinline_cli`

`coinline --help`

## Features

- Set and get the current wallet configuration
- Get the Native Segwit wallet balance
- Retrieve the next unused Native Segwit receiving address and display the Bitcoin URI as a QR code
- Get the transaction history for the wallet
- Sign, send and broadcast transactions to an Electrum server
    - With a file workflow for Coldcard and Keystone
    - Directly, with a Ledger
- Set the Electrum Client. Extremely error prone. Not recommended unless you run your own Electrum server
- Scan for small UTXOs to manage your dust
- Set the UTXO scanning gap between 1 and 50

## Limitations

- Only Native Segwit addresses are currently supported. There is no plan to support Legacy or Nested Segwit addresses. Future support for Taproot functionality may be taken into consideration.
- Only single-signers are currently supported, but multi signature support is next in the queue
- Requests are not routed through Tor, but limited support for Tor may be added in the future

## Roadmap

1. Add multisignature accounts
2. Add or test support for Specter DYI. The code may already work.
3. Support Tor routing
