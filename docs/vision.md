# Vision

EDW is the last ethereum wallet you will ever need.

Notable features include:
- private by design
- simple, legible UX
- encrypted at rest
- easy compartmentalization
  via profiles, or different data dirs
- blazingly fast

## Private by Design

Your data should be yours.
Even when using the CLI only minimal information is shown at a time.
No credentials flashing by on accident, or keys being exposed.

## Encrypted at rest, thats how we do best

All data is encrypted at rest;
And encryption bounds allow for shared access to network caches (such as previously indexed data),
while profile-specific storage is kept seperate.

## Privacy as a first class citizen

Using privacy tooling should be easy;
EDW comes with first class primitives to facilitate managing your funds, shielding, unshielding, and the like.

## Profiles, Accounts, and Spaces

A space is a collection of profiles;
A profile is a collection of accounts;
And an account is a particular method of holding funds, could be an EOA, a smart contract, or an in-protocol balance (like TC)
