# Vocabulary

This document aims to outline used vocabulary, and its definitions.

## Account

An arbitrary construct that could mean "an ethereum wallet" but also "a collection of ethereum wallets" or "a set of notes held in a protocol".

## Asset

Assets are configured wallet-wide and opted-in to on a per "account" basis.
Prefer 'asset' over 'token' or 'currency'.

### Metadata

Asset information such as **decimals**, **symbol**, and **name** are fetched when the asset is first introduced.

### Balance

The **balance** is the amount of an **asset** an **account** holds.

### Value

The **value** of a **balance** is the amount said balance quotes out to be in the **display currency**.

### Display Currency

The users preferred **asset** to view their estimates in.
This should be properly formatted to the users **locale**

## Network

A network, sometimes referred to as "chain" aims to track a specific network id.
Networks are configured wallet-wide.

### Endpoint

A **network endpoint** is a given RPC or mechanism for connecting to the network.
Each endpoint instance is a single RPC either http, ws, or ipc.
For each Network one Network Endpoint is active at a time to provide a stable source of data.
