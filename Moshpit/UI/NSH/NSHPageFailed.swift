import SwiftUI

/// What a page NSH draws shows when its screen can't run (no surface could
/// be made, or the screen would not run): it says so, and Try again runs it
/// once more. The SwiftUI page each used to fall back to is gone (batch D);
/// ``NSHDiagnostics`` has already logged why.
struct NSHPageFailed: View {
    var retry: () -> Void

    var body: some View {
        ContentUnavailableView {
            Label("This page couldn't open.", systemImage: "exclamationmark.triangle")
        } actions: {
            Button("Try again", action: retry)
                .accessibilityIdentifier("nsh-page-retry")
        }
    }
}

/// ``NSHPageFailed`` as a sheet's page: under a bar of its own with
/// Cancel, which closes the sheet. A sheet whose screen can't run showed
/// only Try again, with no way to close it but a swipe.
struct NSHSheetFailed: View {
    var retry: () -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            NSHPageFailed(retry: retry)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Cancel") { dismiss() }
                            .accessibilityIdentifier("nsh-page-cancel")
                    }
                }
        }
    }
}
