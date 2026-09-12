# Security Policy

Moshpit holds SSH keys, passwords and live terminal sessions, so security
reports take priority over every other kind of issue.

## Reporting a vulnerability

Please do not open a public issue for a security problem. Email
cluas@live.cn with:

- what the issue is and where it lives (file, screen or protocol layer),
- steps or a proof of concept that reproduces it,
- the app version shown in Settings and the iOS version.

You will get an acknowledgement as soon as possible, normally within a few
days, and a fix or mitigation plan once the report is confirmed. Fixed versions
ship through the App Store; the changelog credits reporters who want to be
named.

## Scope

The iOS app (every target in this repository) and the push relay under
`push-relay/`. The hosts you connect to, tmux, mosh-server and herdr are their
own projects with their own security contacts.
