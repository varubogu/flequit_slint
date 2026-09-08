import Foundation

/// Process entry point for the iOS app.
///
/// Slint runs on the winit backend on iOS, and winit's iOS backend calls
/// `UIApplicationMain` itself from `EventLoop::run_app`. That means the app must
/// *not* have a Swift `@main` type or an app delegate of its own: there would be
/// two competing owners of the run loop.
///
/// So the C `main` lives here. It installs the lifecycle observers — which only
/// need `NotificationCenter`, available before the application object exists —
/// and then hands control to Rust, which does not return.
@_cdecl("main")
func flequitMain(argc: Int32, argv: UnsafeMutablePointer<UnsafeMutablePointer<CChar>?>) -> Int32 {
    LifecycleForwarder.install()
    return flequit_ios_main()
}
