// When mosh-live stops the server it started: the app's rules
// (MoshServerReap.swift) where the tree being checked has them; otherwise the
// rule SessionHub ran before them, so a build from the code before a fix
// shows what that code did. build.sh sets the flags from the tree it builds.

#if MOSHPIT_REAP_RETURN_PATH
let returnPathRule = "the app's (MoshServerReap.onReturnPathDead)"
func shouldStopOnReturnPathDead(heardFromServer: Bool?) -> Bool {
    MoshServerReap.onReturnPathDead(heardFromServer: heardFromServer ?? false)
}
#else
let returnPathRule = "SessionHub before B165 (every report stops the server)"
func shouldStopOnReturnPathDead(heardFromServer: Bool?) -> Bool { true }
#endif

#if MOSHPIT_REAP_TEARDOWN
let userCloseRule = "the app's (MoshServerReap.onTeardown)"
func shouldStopOnUserClose(contacted: Bool) -> Bool {
    MoshServerReap.onTeardown(.userClosed, contacted: contacted)
}
#else
let userCloseRule = "SessionHub's stop() before B164 (only a server never contacted)"
func shouldStopOnUserClose(contacted: Bool) -> Bool { !contacted }
#endif

#if MOSHPIT_RETURN_PATH_HEARD
/// The transport's callback: whether the server was ever heard from.
func returnPathCallback(_ note: @escaping (Bool?) -> Void) -> (Bool) -> Void { { note($0) } }
#else
/// The transport's callback before B165 said nothing about contact.
func returnPathCallback(_ note: @escaping (Bool?) -> Void) -> () -> Void { { note(nil) } }
#endif

let appRules = "return path: \(returnPathRule); user close: \(userCloseRule)"
