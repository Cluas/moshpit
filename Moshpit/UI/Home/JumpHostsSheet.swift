import SwiftUI

/// "Jump hosts (N)" on Home: the connections that are jump hosts only
/// (they aren't listed on Home, but every Connect via offers them). A tap
/// edits one in the normal form; a swipe deletes it, after the same
/// confirmation as Home's, which names the connections that go through it.
struct JumpHostsSheet<Form: View>: View {
    @Environment(\.dismiss) private var dismiss
    let store: ConnectionStore
    let form: (ServerConnection) -> Form
    let delete: (ServerConnection) -> Void

    @State private var editing: ServerConnection?
    @State private var deleting: ServerConnection?

    private var jumpHosts: [ServerConnection] {
        store.connections.filter { $0.jumpHostOnly == true }
    }

    var body: some View {
        NavigationStack {
            List {
                ForEach(jumpHosts) { host in
                    Button { editing = host } label: {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(host.displayName).foregroundStyle(.primary)
                            Text(verbatim: "\(host.username)@\(host.host)")
                                .font(.footnote.monospaced()).foregroundStyle(.secondary)
                        }
                        .frame(minHeight: 44, alignment: .leading)
                    }
                    .swipeActions {
                        Button("Delete", role: .destructive) { deleting = host }
                    }
                }
            }
            .overlay { if jumpHosts.isEmpty { Color.clear.onAppear { dismiss() } } }
            .navigationTitle(Text("Jump hosts (\(jumpHosts.count))"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
            }
            .appSheet(item: $editing) { form($0) }
            .confirmationDialog(
                "Delete \(deleting?.displayName ?? String(localized: "connection"))?",
                isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
                titleVisibility: .visible, presenting: deleting
            ) { host in
                Button("Delete", role: .destructive) { delete(host); deleting = nil }
                Button("Cancel", role: .cancel) { deleting = nil }
            } message: { host in
                Text(verbatim: ConnectionDeleteWords.message(for: host, in: store.connections))
            }
        }
    }
}
