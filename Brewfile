# Contributor toolchain for Moshpit.
#
# Run `brew bundle` from the repo root to install everything needed to
# generate the Xcode project and run the linters/formatter in one step.
#
# Xcode itself is not managed here — install it from the App Store.

# Generates Moshpit.xcodeproj from project.yml (the source of truth).
brew "xcodegen"

# Lint + format. Configured by .swiftlint.yml and .swiftformat at the repo root.
brew "swiftlint"
brew "swiftformat"
