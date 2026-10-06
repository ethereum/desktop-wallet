# Storage

EDW keeps one encrypted store per network.
It holds that network's endpoints and assets, the keyring, the profiles and their accounts, and the protocol caches the profiles share.

## Layout

```sh
<data-dir>/<network>/                 # "mainnet", "sepolia", "local", or the numeric network id

network          / network            # Network { id, native_asset }
                 / endpointConfigs    # Vec<NetworkEndpointConfig>
                 / activeEndpoint     # name of the active endpoint config
                 / assets             # Vec<Asset>: the network's ERC-20 and ERC-1155 tokens
keyring          / mnemonics          # indices of the stored recovery phrases
                 / mnemonic:{m}       # one recovery phrase
profiles         / index              # Vec<ProfileRecord { m, x, name }>
profile:{m}:{x}  / accounts           # Vec<AccountRecord { id, kind, label, assets }>
                 / assets             # assets enabled for every account of the profile
                 / account:{id}:...   # sync state of one account
cache:{protocol} / cursor             # last indexed block
                 / chunk:{n}          # indexed events for one block span
```

## Keyring

Recovery phrases are stored one per record, indexed by `m`.
Keys derived from a phrase are never stored.

## Profiles

Profiles are keyed by their recovery phrase `m` and profile index `x`, the BIP-44 `account'` leaf.
Several profiles may share `m` under different `x`.

## Accounts

Accounts are stored per profile, one entry per derivation branch.
Each entry keeps the public side of its branch, an address or meta-address, so reading accounts never needs the phrase.

```sh
Address { index }                       # m/44'/60'/x'/0/i, where i = 0 is the identity anchor
Stealth { scheme }                      # m/44'/60'/x'/5564'/scheme'/{0,1}, spending and viewing
TornadoCash                             # m/29795'/1'/x'/{0',1'}/deposit', nullifier and salt
Railgun                                 # m/44'/1984'/0'/0'/x' and m/420'/1984'/0'/0'/x', BabyJubJub
SmartAccount { implementation, index }  # m/44'/60'/x'/i'/j'
```

Accounts that sync keep their state under `account:{id}`:

- Stealth: the payments found in announcements.
- TornadoCash: its notes (pool, deposit index, commitment, leaf index, spent). A new note takes the next unused `deposit'`.

## Assets

`network / assets` lists the assets configured for the network.
The native asset is not listed and is always enabled.

Enabled assets are stored on the profile, on the account, or both.
A balance overview queries an account only for the native asset and the assets enabled for it or its profile.

## Caches

Each protocol (`tornado-cash`, `railgun`, ...) gets one cache per network, shared by every account on it.
Events are written in numbered chunks behind a cursor, since stored keys cannot be listed.

## Encryption

A random data key, wrapped by the password slot, seals every record.
The backend sees only blinded record names, never a scope or key.
Each `profile:{m}:{x}` scope is self-contained, so a profile can later get its own password.
