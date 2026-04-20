import SwiftUI

struct DriveCkCard<Content: View>: View {
    @Environment(\.colorScheme) private var colorScheme

    var padding: CGFloat = 18
    @ViewBuilder var content: Content

    var body: some View {
        content
            .padding(padding)
            .background {
                RoundedRectangle(cornerRadius: 24, style: .continuous)
                    .fill(.thinMaterial)
            }
            .overlay {
                RoundedRectangle(cornerRadius: 24, style: .continuous)
                    .strokeBorder(Color.primary.opacity(colorScheme == .dark ? 0.18 : 0.08), lineWidth: 1)
            }
            .shadow(color: .black.opacity(colorScheme == .dark ? 0.10 : 0.04), radius: 10, x: 0, y: 4)
    }
}

struct DriveCkStatusBadge: View {
    var text: String
    var tint: Color

    var body: some View {
        Text(text)
            .font(.caption.weight(.semibold))
            .foregroundStyle(tint)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(tint.opacity(0.12), in: Capsule())
    }
}

struct DriveCkCountBadge: View {
    var text: String

    var body: some View {
        Text(text)
            .font(.caption2.weight(.semibold))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(.secondary.opacity(0.10), in: Capsule())
    }
}

struct DriveCkMetricTile: View {
    @Environment(\.colorScheme) private var colorScheme

    var title: String
    var value: String
    var secondary: String? = nil
    var tint: Color = .accentColor
    var monospaced = false

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack(spacing: 6) {
                Circle()
                    .fill(tint.opacity(0.28))
                    .frame(width: 7, height: 7)
                Text(title)
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.secondary)
            }
            Text(value)
                .font(monospaced ? .callout.weight(.semibold) : .title3.weight(.semibold))
                .fontDesign(monospaced ? .monospaced : nil)
                .lineLimit(1)
                .minimumScaleFactor(0.82)
            if let secondary {
                Text(secondary)
                    .font(monospaced ? .caption : .caption)
                    .fontDesign(monospaced ? .monospaced : nil)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .background(
            Color.primary.opacity(colorScheme == .dark ? 0.10 : 0.035),
            in: RoundedRectangle(cornerRadius: 16, style: .continuous)
        )
    }
}
