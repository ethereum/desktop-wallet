# Storage

EDW keeps one encrypted store per network.
It holds that network's endpoints, the keyring, the profiles and their accounts, and the protocol caches the profiles share.

## Layout

```sh
<data-dir>/<network>/                 # "mainnet", "sepolia", "local", or the numeric network id

network          / network            # Network { id, native_asset }
                 / endpointConfigs    # Vec<NetworkEndpointConfig>
                 / activeEndpoint     # name of the active endpoint config
keyring          / mnemonics          # indices of the stored recovery phrases
                 / mnemonic:{m}       # one recovery phrase
profiles         / index              # Vec<ProfileRecord { m, x, name }>
profile:{m}:{x}  / accounts           # Vec<AccountRecord>
                 / account:{id}:...   # sync state of one account
cache:{protocol} / cursor             # last indexed block
                 / chunk:{n}          # indexed events for one block span
```

## Keyring

The keyring holds recovery phrases, indexed by `m`.
Keys are derived from a phrase on use and never stored.

## Profiles

A profile is a recovery phrase `m` and a profile index `x`, the BIP-44 `account'` leaf.
Profiles may share a phrase under different `x`.

## Accounts

An account is one branch of a profile, derived from its phrase at `x`:

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

## Caches

A cache holds the indexed events of one protocol (`tornado-cash`, `railgun`, ...), shared by every account on the network.
Events are written in numbered chunks behind a cursor, since stored keys cannot be listed.

## Encryption

A random data key, wrapped by the password slot, seals every record.
The backend sees only blinded record names, never a scope or key.
Each `profile:{m}:{x}` scope is self-contained, so a profile can later get its own password.
