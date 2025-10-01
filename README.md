# Althea L1 Gasless relayer

The Althea L1 gasless relayer is an ecosystem wide system for allowing a market of relayers to submit 'gasless' transactions. These are transactions where the user does not have ALTHEA but wishes to pay their transaction fee in another token.
The relayer application queries various transaction sources and relays transactions when they are available with a required profit margin between the provided payment token and the value of ALTEHA required to actually execute the transaction.

## Download Instructions

Download the appropriate binary for your platform from the [releases page](https://github.com/AltheaFoundation/althea-l1-relayer/releases)

## Build Instructions (Ubuntu)

1. Pull the repo and build. You might need to install dependencies if not already done.

```
apt install rustup
apt install build-essential
rustup default stable
git clone https://github.com/AltheaFoundation/althea-l1-relayer
cd althea-l1-relayer
cargo build
```

## Setup Instructions (Linux)

1. Move the built binary to your `/usr/bin`, and test.

```
mv BINARY_DOWNLOAD_PATH /usr/bin
althea-l1-relayer -- help
```

## Use Instructions (all platforms)

1. You will need an ETH wallet with a bit of $ALTHEA in it (1-2 $ALTHEA should be enough). For security and safety, create a new wallet in your choice of app (e.g. Metamask) and prepare it with the $ALTHEA.

2. /Input the private key from the ETH wallet you created in Step 3 as a flag.

```
althea-l1-relayer --private-key <64-char ETH private key>
```

3. Read the warning / terms and make sure you understand them, then start the relayer.

```
althea-l1-relayer --agree --private-key <64-char ETH private key>
```
