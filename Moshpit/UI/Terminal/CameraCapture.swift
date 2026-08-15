import SwiftUI
import UIKit

/// The camera as an attach-image source: photograph a whiteboard, a sketch,
/// another screen, and hand it to the agent. A thin wrapper over
/// `UIImagePickerController` — PHPicker has no capture mode, and the full
/// AVFoundation route buys nothing here but surface area.
///
/// The capture comes back as JPEG bytes and runs through the SAME pipeline
/// as picked images (scale, EXIF/GPS strip, upload) — the camera is a
/// source, not a second code path.
struct CameraCaptureView: UIViewControllerRepresentable {
    /// JPEG bytes of the shot, or nil on cancel.
    let onFinish: (Data?) -> Void

    static var isAvailable: Bool {
        UIImagePickerController.isSourceTypeAvailable(.camera)
    }

    func makeUIViewController(context: Context) -> UIImagePickerController {
        let picker = UIImagePickerController()
        picker.sourceType = .camera
        picker.delegate = context.coordinator
        return picker
    }

    func updateUIViewController(_ picker: UIImagePickerController, context: Context) {}

    func makeCoordinator() -> Coordinator { Coordinator(onFinish: onFinish) }

    final class Coordinator: NSObject, UIImagePickerControllerDelegate,
                             UINavigationControllerDelegate {
        let onFinish: (Data?) -> Void
        init(onFinish: @escaping (Data?) -> Void) { self.onFinish = onFinish }

        func imagePickerController(
            _ picker: UIImagePickerController,
            didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]) {
            // Near-lossless here; the attachment pipeline does the real
            // scale + recompress, and double-compressing a whiteboard shot
            // at 0.95 costs nothing visible.
            let data = (info[.originalImage] as? UIImage)?.jpegData(compressionQuality: 0.95)
            onFinish(data)
        }

        func imagePickerControllerDidCancel(_ picker: UIImagePickerController) {
            onFinish(nil)
        }
    }
}
