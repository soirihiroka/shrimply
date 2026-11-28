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
            let numberPicker = (view as? NSStackView)?.arrangedSubviews.last is NSStepper
            AppKitControl(view: view, fullWidth: false)
                .id(numberPicker ? nil : ObjectIdentifier(view))
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

    func updateNSView(_ nsView: NSView, context: Context) {
        guard nsView !== view, let current = nsView as? NSStackView,
              let next = view as? NSStackView else { return }
        // Keep the numeric field editor and stepper tracking alive across commits.
        for (existing, replacement) in zip(current.arrangedSubviews, next.arrangedSubviews) {
            guard let control = existing as? NSControl, let value = replacement as? NSControl
            else { continue }
            if control.doubleValue != value.doubleValue {
                control.doubleValue = value.doubleValue
            }
            control.isEnabled = value.isEnabled
        }
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSView, context: Context) -> CGSize? {
        let size = nsView.fittingSize
        let width = proposal.width.flatMap { $0.isFinite ? $0 : nil } ?? size.width
        return CGSize(width: max(0, fullWidth ? width : min(width, size.width)),
                      height: max(0, size.height))
    }
}
