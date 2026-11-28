import AppKit
import SwiftUI

@MainActor
@_cdecl("shrimply_settings_form_create")
public func createSettingsForm() -> UnsafeMutableRawPointer {
    let host = NSHostingView(rootView: SettingsForm())
    host.sizingOptions = []
    return Unmanaged.passRetained(host).toOpaque()
}

@MainActor
@_cdecl("shrimply_settings_form_update")
public func updateSettingsForm(
    _ host: UnsafeRawPointer, _ page: UnsafeRawPointer, _ sections: UnsafeRawPointer
) {
    let view = Unmanaged<NSHostingView<SettingsForm>>.fromOpaque(host).takeUnretainedValue()
    view.rootView = SettingsForm(
        page: Unmanaged<NSString>.fromOpaque(page).takeUnretainedValue() as String,
        sections: Unmanaged<NSArray>.fromOpaque(sections).takeUnretainedValue() as? [[Any]] ?? []
    )
}

private struct SettingsForm: View {
    var page = ""
    var sections: [[Any]] = []

    var body: some View {
        Form {
            ForEach(sections.indices, id: \.self) { index in
                let section = sections[index]
                Section {
                    ForEach(section.indices.dropFirst(), id: \.self) { row in
                        SettingsRow(parts: section[row] as? [Any] ?? [])
                    }
                } header: {
                    if let title = section.first as? String, !title.isEmpty {
                        Text(title)
                    }
                }
            }
        }
        .formStyle(.grouped)
        .id(page)
    }
}

private struct SettingsRow: View {
    let parts: [Any]

    var body: some View {
        if let view = parts.first as? NSView {
            AppKitControl(view: view, fullWidth: true).id(ObjectIdentifier(view))
        } else {
            let title = parts.first as? String ?? ""
            let subtitle = parts.dropFirst().first as? String ?? ""
            if title.isEmpty && subtitle.isEmpty {
                HStack {
                    Spacer()
                    accessory
                }
            } else {
                LabeledContent {
                    accessory
                } label: {
                    if !title.isEmpty {
                        Text(title)
                    }
                    if !subtitle.isEmpty {
                        Text(subtitle)
                    }
                }
            }
        }
    }

    @ViewBuilder
    private var accessory: some View {
        let value = parts.dropFirst(2).first
        if let view = value as? NSView {
            AppKitControl(view: view, fullWidth: false).id(ObjectIdentifier(view))
        } else if let text = value as? String {
            Text(text)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.trailing)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: 360, alignment: .trailing)
                .textSelection(.enabled)
        }
    }
}

private struct AppKitControl: NSViewRepresentable {
    let view: NSView
    let fullWidth: Bool

    func makeNSView(context: Context) -> NSView { view }
    func updateNSView(_ nsView: NSView, context: Context) {}

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSView, context: Context) -> CGSize? {
        let size = nsView.fittingSize
        let width = proposal.width ?? size.width
        return CGSize(width: fullWidth ? width : min(width, size.width), height: size.height)
    }
}
