# TestFlight — getting a build to your colleagues

Everything on the machine side is ready: Release builds clean for device, the
archive script exists, and the screenshots are already at the size App Store
Connect demands. What's left needs your Apple ID, which nothing here can do on
your behalf.

## What's blocking (all of it needs you, once)

This Mac has **no signing identity and no App Store Connect key** — checked:
`security find-identity -v -p codesigning` returns zero identities, and there
are no keys under `~/.appstoreconnect/private_keys/`. So:

1. **Xcode ▸ Settings ▸ Accounts** → add the Apple ID of the (now approved)
   Developer Program account.
2. Fill in the Team ID:
   ```sh
   ./scripts/team-id.sh          # prints the 10-character ID
   # put it in Signing.xcconfig → DEVELOPMENT_TEAM =
   git update-index --skip-worktree Signing.xcconfig   # keep it out of git
   ```
3. **App Store Connect → Apps → +** → New App
   - Platform: iOS · Name: **Moshpit** · Primary language: English (U.S.)
   - Bundle ID: **`com.cluas.moshpit`** — register it first under
     Certificates, Identifiers & Profiles if it isn't in the dropdown. It needs
     these capabilities, both already in the entitlements:
     **App Groups** (`group.com.cluas.moshpit`, for the Live Activity) and
     **Push Notifications**.
   - SKU: anything stable, e.g. `moshpit-ios`

## Then: archive and upload

```sh
./scripts/release-archive.sh
```

It archives Release, checks that the app and the `MoshpitIsland` extension
carry the identical `CFBundleVersion` (a mismatch is an upload rejection), and
exports a signed `build/AppStore/Moshpit.ipa`. Upload it with Xcode's Organizer
(**Window ▸ Organizer ▸ Distribute App**), or from the CLI once you have an
App Store Connect API key.

Build numbers come from `git rev-list --count HEAD`, so they always increase.
To re-upload without a dummy commit: `MOSHPIT_BUILD=n ./scripts/release-archive.sh`.

## Internal vs external testers

For colleagues, **internal** is the fast path:

| | Internal | External |
|---|---|---|
| Who | Up to 100 people you add as Users in App Store Connect | Up to 10,000, invited by email or a public link |
| Beta App Review | **Not required** — testable minutes after processing | Required for the first build (usually under a day) |
| Setup | Add each colleague as a User (Developer or higher), tick them into the internal group | Create a group, add a public link |

So: add colleagues under **Users and Access**, then in **TestFlight → Internal
Testing** add them to a group and tick the build. They get an email; the
TestFlight app does the rest.

## Export compliance — already answered

`ITSAppUsesNonExemptEncryption: true` is in the Info.plist with the reasoning
recorded next to it, so uploads won't park on "Missing Compliance". The first
submission of a version still asks the follow-up questions in App Store
Connect; the honest answer is standard algorithms, nothing proprietary.

## What testers should know

Moshpit connects to *their own* servers — a colleague with no SSH host will see
an empty app. Point them at the same demo host the reviewer gets
([`app-review.md`](app-review.md)), or tell them to add their own machine.

One caveat worth putting in the tester notes: if they run a TUN-mode proxy
(Clash and friends), it hijacks DNS into a fake-IP range and SSH to a hostname
fails. Plain Wi-Fi or cellular is the clean test.
