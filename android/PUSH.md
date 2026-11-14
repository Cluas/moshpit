# Push on Android: what linking FCM takes

Moshpit for Android posts its own notifications today (`Notifier.java`, the
`agents` and `sessions` channels) but receives no remote push: Google's
Firebase Cloud Messaging (FCM) is the only push service Play phones have,
and it needs Google Play services, which are closed source. The build keeps
a hook instead of the library: `Notifier.PushHook` (this build's is
`NO_PUSH`: no token), and Set up this host says pairing needs it
(`crates/android/src/app/host_setup.rs`, `NO_PUSH`). This page is the list
of what linking FCM needs, in the order to do it. The decisions behind it
are `docs/nsh/DESIGN.md` §3.2 (推送扩展) and §5 (许可证).

## 1. Firebase and its config

- A Firebase project with an Android app `com.cluas.moshpit`, the upload
  key's and Play App Signing's SHA-256 fingerprints added to it.
- Its `google-services.json` at `android/app/google-services.json`. It holds
  the project's API key and sender id: not a secret, but not the public
  repository's either. Keep it out of git (`android/.gitignore`) and out of
  `export-oss.sh`'s `PUBLIC` list; CI writes it from a secret
  (`MOSHPIT_GOOGLE_SERVICES_JSON_BASE64`) before `android/ci.sh` runs, the
  way the upload key is written now.
- The relay gets an FCM service account (HTTP v1 API, `firebase.messaging`
  scope) beside its APNs `.p8`.

## 2. The Gradle build

- Two product flavors, so the source stays buildable without Google's
  libraries: `oss` (today's app, no push; what `android/build.sh` builds)
  and `play` (FCM; what the Play bundle is). Only `play` applies
  `com.google.gms.google-services` and depends on
  `platform("com.google.firebase:firebase-bom:<pinned>")` and
  `com.google.firebase:firebase-messaging`. Pin both in
  `android/gradle/libs.versions.toml`; `android/ci.sh` then builds
  `:app:bundlePlayRelease`.
- `play`'s manifest adds the service:

  ```xml
  <service android:name="com.cluas.moshpit.push.MessagingService" android:exported="false">
      <intent-filter><action android:name="com.google.firebase.MESSAGING_EVENT" /></intent-filter>
  </service>
  ```

  `POST_NOTIFICATIONS` is already declared; the app asks for it at the
  first pairing (Android 13+), not at launch.

## 3. The token

`MessagingService.onNewToken` (and `FirebaseMessaging.getToken()` at start)
hands the token to the core through the hook (`PushHook.token()`, a new
`Core.pushToken(token)`). It takes the place of the APNs token everywhere
the pairing uses one:

- `/v1/mint` (`moshpit_seal::token::MintRequest`) sends it, with a new
  `kind: "fcm"` field (APNs when absent, so iOS keeps working). The relay
  routes by kind.
- The send token's HMAC (`token::mint`, `moshpit-send-v2|lower(token)|…`)
  lower-cases A–F in the token. That is harmless for hex APNs tokens and
  only for the MAC, but an FCM token is case-sensitive base64-ish text: the
  relay must route on the token as registered, never on the lower-cased
  form, and the MAC should name the kind (`moshpit-send-v3|fcm|token|…`) so
  an APNs token and an FCM token can never mint the same send token.
- The host's `push.d/<conn>.conf` (`moshpit_install::push_conf`) writes
  `APNS_TOKEN=` / `APNS_ENV=`; it gains `PUSH_KIND=fcm` and the token goes
  in the same field (the sender script passes both to the relay).
- The pairings are kept as iOS keeps them (`push-pairings.json`,
  `PushPairing`: the secret, the send token, the token it was minted over),
  in the Keystore-wrapped keychain, so a re-mint after `onNewToken` works
  like iOS's after a new APNs token (`PushPairing::needs_mint`).

## 4. Receiving: the sealed envelope, as the iOS extension opens it

The host seals each status with the pairing's secret (`moshpit-push.sh`,
format v1: AES-256-CBC + HMAC-SHA256, encrypt-then-MAC); the relay never
sees the plaintext. On iOS the Notification Service Extension opens it
(`NotificationService.didReceive`); on Android the same flow is already in
Rust, `moshpit_seal::receive::receive`:

1. The relay sends a **data-only, high-priority** FCM message (no
   `notification` block, which Android would show without the app), whose
   data carries the envelope under `mp` as the APNs payload does
   (`{"mp": {"v": 1, "iv", "ct", "mac"}}`; `notification::ENVELOPE_KEY`).
2. `MessagingService.onMessageReceived` passes the data as JSON to a new
   JNI entry (`Core.receivePush(json)`), which may run in a process the
   system started only for this: it must not need the activity or the
   controller, only the stores. It reads the pairings, the standing prompts
   (`PushStandingFile`) and the notification preferences (`prefs::Prefs`),
   calls `receive(payload, pairings, standing, prefs, language, now)`, writes
   back what changed, and posts the `Delivery`:
   - `Card`: the opened, rendered card (`Notifier.post` on `agents`, its tag
     per connection and pane, its tap `moshpit://connection/<id>?pane=…`);
   - `Fallback`: the relay's generic text, no actions;
   - `AsReceived`: not ours.
3. Before the first unlock after a reboot the keychain (credential-encrypted
   storage) can't be read: the card is the fallback, as on a locked iPhone
   whose App Group file is protected.
4. The diagnostics lines `receive` returns go to the Recent Log ring as iOS's
   do.

## 5. The license: GPL-3.0 §7 additional permission

The app is GPL-3.0-only; Google Play services and Firebase are not
GPL-compatible. Before a build with them is distributed, `LICENSE` gets this
paragraph after the license text (Cluas is the only copyright holder, so he
can grant it; `DESIGN.md` §5 decided it):

> **Additional permission under GNU GPL version 3 section 7**
>
> If you modify this Program, or any covered work, by linking or combining
> it with the Google Play services client libraries or the Firebase SDK for
> Android (or a modified version of those libraries), containing parts
> covered by the terms of the Android Software Development Kit License
> Agreement or the Google APIs Terms of Service, the licensors of this
> Program grant you additional permission to convey the resulting work.
> Corresponding Source for a non-source form of such a combination shall
> not include the source code for the parts of those libraries used as well
> as that of the covered work.

`NOTICES.md` then lists the Firebase libraries in the `play` flavor with
their licenses (Apache-2.0 for the SDK's open parts, Google's terms for
Play services).

## 6. What stays as it is

The relay's envelope format, the host's sender and stamp scripts, the iOS
app and its extension: FCM is one more route at the relay and one more
receiver on the phone. The `oss` flavor keeps working without any of it.
