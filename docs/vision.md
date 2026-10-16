# Vision

EDW is meant to be Ethereum's _wallet of last resort_. It has two key priorities as an Ethereum wallet:

1. **Dependency minimization**: This local software and a connection to ethereum nodes is all that's needed to reliably operate. Any other dependencies are optional conveniences that don't block critical wallet functionality.

2. **Privacy by Default**: Users onchain actions and balances are maximally private, meaning unlinked and anonymous to any outside observer.

**Those two properties are the product.** Speed, convenience, a simple/legible UX, and minimzation of tx fees are also important aspects to optimize around, but not when they conflict with Property 1 or Property 2.

Finally **Security** is an overarching priority which supercedes all other goals. Wallet software that is insecure is actively harmful and any benefits or properties are not relevant if the software doesn't secure your funds and keys properly.

## Dependency minimization

**This local software and a connection to ethereum nodes is all that's needed to reliably operate.**

One asterisk: until Hegota and EIP-8141, one dependency besides a node connection has to be swallowed -- access to at least one 4337 bundler endpoint for UserOp submission. That endpoint should be a replaceable dumb pipe for UserOp submission and nothing else. Several open public endpoints exist, so the wallet can keep redundancy across them and still submit when any given one is down.

Besides the asterisk, every single other endpoint or service we rely on should be optional and non-critical. If that endpoint is unexpectedly down or malicious the wallet still operates (maybe slower or with less auxiliary data and conveniences, but critical functionality still works)

## Privacy by Default

**Users onchain actions and balances are maximally private, meaning unlinked and anonymous to any outside observer.** 

In practice, this means:

- **Deep integration with shielded pool solutions** so that unshielding freshly anonymized funds, before doing transfers or taking any new onchain action, is the standard workflow.
- **Careful analysis of traffic and fingerprint** whenever user does wallet interaction or transaction flows. The goal is that network observers learn/link nothing more than what is visible onchain. 

**Profiles** are the compartmentalization technique that makes this practical and legibile for users. The goal of the **Profile** is to make it so that a user who doesn't understand intricacies of onchain privacy can simply follow the default wallet UX (which gives them privacy) without losing their ability to understand their balances and activity history. Thanks to the **Profile** using a fresh unlinked address for each onchain action is the default, and reusing addresses / linking activity is a custom anti-pattern that a user must explicitly seek out.
