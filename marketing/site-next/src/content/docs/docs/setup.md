---
title: "Set up a connection"
description: "Everything between tapping ＋ and having a live shell: the fields, the key, the fingerprint prompt, the multiplexer choice, and what Moshpit does when the host turns out not to have tmux, herdr or mosh-server. About a minute if the host is ready."
---

## Add the host

Name, Host, Port, Username. **Save** turns on the moment Name and Host both have text — everything else has a working default.

## Pick how you sign in

A password, a key you generate on the phone, or a PEM you paste. The secret goes to the iOS Keychain; the connection record only holds a reference to it.

## Check the fingerprint

The first handshake stops and shows the host key with the `ssh-keygen` line to compare it against. Trust, or Cancel.
