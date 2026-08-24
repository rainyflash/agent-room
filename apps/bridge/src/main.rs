mod secure_storage;

fn main() {
    let _signing_identities =
        secure_storage::OsDeviceSigningIdentityStore::system("dev.agent-room.bridge");
    let _credentials = secure_storage::OsDeviceCredentialVault::system("dev.agent-room.bridge");
}
