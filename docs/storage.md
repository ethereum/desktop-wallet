# Storage

EDW keeps one encrypted store per network.
It holds that network's RPC configs, the keyring, the profiles, and the protocol caches the profiles share.

## Layout

```sh
<data-dir>/<network>/                 # "mainnet", "sepolia", "local", or the numeric network id

network          / network            # Network { id, native_asset }
                 / rpcConfigs         # Vec<RpcConfig>
                 / activeRpcConfig    # name of the active RPC config
keyring          / mnemonics          # indices of the stored recovery phrases
                 / mnemonic:{m}       # one recovery phrase
profiles         / index              # Vec<ProfileRecord { m, x, name }>
profile:{m}:{x}  / pointer            # (m, x)
                 / optedIn            # Vec<Address>, extra ERC-20s the profile tracks
                 / optedOut           # Vec<Address>, default ERC-20s the profile does not track
                 / vaults             # Vec<VaultRecord { tag, id }>, in the order added
  vault:{id}     / ...                # one vault's own records, inside its profile's scope
                 / address            # DerivedAddress { address, i }, for an `address` vault
cache:{protocol} / cursor             # last indexed block
                 / chunk:{n}          # indexed events for one block span
```

## Keyring

The keyring holds recovery phrases, indexed by `m`.
Keys are derived from a phrase on use and never stored.

## Profiles

A profile is a recovery phrase `m` and a profile index `x`, the BIP-44 `account'` leaf.
Profiles may share a phrase under different `x`.

A profile derives its keys at `x`:

```sh
Address { index }                       # m/44'/60'/x'/0/i, where i = 0 is the identity anchor
Stealth { scheme }                      # m/44'/60'/x'/5564'/scheme'/{0,1}, spending and viewing
TornadoCash                             # m/29795'/1'/x'/{0',1'}/deposit', nullifier and salt
Railgun                                 # m/44'/1984'/0'/0'/x' and m/420'/1984'/0'/0'/x', BabyJubJub
SmartAccount { implementation, index }  # m/44'/60'/x'/i'/j'
```

## Caches

A cache holds the indexed events of one protocol (`tornado-cash`, `railgun`, ...), shared by every vault on the network.
Events are written in numbered chunks behind a cursor, since stored keys cannot be listed.

## Encryption

A random data key, wrapped by the password slot, seals every record.
The backend sees only blinded record names, never a scope or key.
Each `profile:{m}:{x}` scope is self-contained, so a profile can later get its own password.
