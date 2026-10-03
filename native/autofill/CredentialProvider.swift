import AppKit
import AuthenticationServices

private struct EmailCode: Decodable {
    let id: String
    let code: String
    let domain: String
    let label: String
    let expiresAtMs: Double

    static func available() -> [EmailCode] {
        guard let container = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: "TQV87WLXK3.studio.margin.mail"
        ),
        let data = try? Data(contentsOf: container.appendingPathComponent("email-otp.json")),
        let codes = try? JSONDecoder().decode([EmailCode].self, from: data) else {
            return []
        }
        let now = Date().timeIntervalSince1970 * 1000
        return codes.filter {
            !$0.id.isEmpty && !$0.code.isEmpty && $0.code.count <= 32
                && !$0.domain.isEmpty && $0.expiresAtMs > now
                && $0.expiresAtMs <= now + 180_000
        }.sorted { $0.expiresAtMs > $1.expiresAtMs }
    }

    func matches(_ service: ASCredentialServiceIdentifier) -> Bool {
        let host: String
        if service.type == .URL {
            guard let urlHost = URL(string: service.identifier)?.host else { return false }
            host = urlHost.lowercased()
        } else if service.type == .domain {
            host = service.identifier.lowercased()
        } else {
            return false
        }
        let senderDomain = domain.lowercased()
        return host == senderDomain || host.hasSuffix("." + senderDomain)
    }
}

@available(macOS 15.0, *)
final class CredentialProviderViewController: ASCredentialProviderViewController {
    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 420, height: 340))
    }

    override func provideCredentialWithoutUserInteraction(for credentialRequest: any ASCredentialRequest) {
        provide(credentialRequest)
    }

    override func prepareInterfaceToProvideCredential(for credentialRequest: any ASCredentialRequest) {
        provide(credentialRequest)
    }

    override func prepareOneTimeCodeCredentialList(for serviceIdentifiers: [ASCredentialServiceIdentifier]) {
        let codes = EmailCode.available()
        let matching = codes.filter { code in serviceIdentifiers.contains { code.matches($0) } }
        let others = codes.filter { code in !serviceIdentifiers.contains { code.matches($0) } }
        let stack = NSStackView()
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 12
        stack.edgeInsets = NSEdgeInsets(top: 20, left: 20, bottom: 20, right: 20)
        stack.translatesAutoresizingMaskIntoConstraints = false

        let title = NSTextField(labelWithString: "Email verification codes")
        title.font = .boldSystemFont(ofSize: 17)
        stack.addArrangedSubview(title)

        if codes.isEmpty {
            let empty = NSTextField(wrappingLabelWithString:
                "No recent codes are available. Enable Email OTP AutoFill in Margin Mail and keep the app unlocked."
            )
            stack.addArrangedSubview(empty)
        } else {
            add(matching, to: stack)
            if !others.isEmpty {
                if !serviceIdentifiers.isEmpty {
                    let label = NSTextField(wrappingLabelWithString:
                        "Other email codes. Select only the code for the website you are using."
                    )
                    label.textColor = .secondaryLabelColor
                    stack.addArrangedSubview(label)
                }
                add(others, to: stack)
            }
        }

        let cancel = NSButton(title: "Cancel", target: self, action: #selector(cancelSelection))
        cancel.bezelStyle = .rounded
        stack.addArrangedSubview(cancel)

        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.documentView = stack
        view.subviews.forEach { $0.removeFromSuperview() }
        view.addSubview(scroll)
        NSLayoutConstraint.activate([
            scroll.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            scroll.topAnchor.constraint(equalTo: view.topAnchor),
            scroll.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            stack.leadingAnchor.constraint(equalTo: scroll.contentView.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: scroll.contentView.trailingAnchor),
            stack.topAnchor.constraint(equalTo: scroll.contentView.topAnchor),
        ])
    }

    private func add(_ codes: [EmailCode], to stack: NSStackView) {
        for code in codes {
            let button = NSButton(
                title: "\(code.code)  ·  \(code.label)",
                target: self,
                action: #selector(selectCode(_:))
            )
            button.identifier = NSUserInterfaceItemIdentifier(code.id)
            button.bezelStyle = .rounded
            button.toolTip = code.domain
            stack.addArrangedSubview(button)
            let domain = NSTextField(labelWithString: code.domain)
            domain.font = .systemFont(ofSize: 11)
            domain.textColor = .secondaryLabelColor
            stack.addArrangedSubview(domain)
        }
    }

    private func provide(_ request: any ASCredentialRequest) {
        guard request.type == .oneTimeCode,
              let identity = request.credentialIdentity as? ASOneTimeCodeCredentialIdentity,
              let id = identity.recordIdentifier,
              let code = EmailCode.available().first(where: { $0.id == id }),
              code.matches(identity.serviceIdentifier) else {
            cancel(with: .credentialIdentityNotFound)
            return
        }
        extensionContext.completeOneTimeCodeRequest(using: ASOneTimeCodeCredential(code: code.code))
    }

    @objc private func selectCode(_ sender: NSButton) {
        guard let id = sender.identifier?.rawValue,
              let code = EmailCode.available().first(where: { $0.id == id }) else {
            cancel(with: .credentialIdentityNotFound)
            return
        }
        extensionContext.completeOneTimeCodeRequest(using: ASOneTimeCodeCredential(code: code.code))
    }

    @objc private func cancelSelection() {
        cancel(with: .userCanceled)
    }

    private func cancel(with code: ASExtensionError.Code) {
        extensionContext.cancelRequest(withError: NSError(domain: ASExtensionErrorDomain, code: code.rawValue))
    }
}
