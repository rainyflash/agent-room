mod http;

pub use http::{
    DnsResolver, HttpsDocumentClient, JsonDocument, NetworkTargetFailure, NetworkTargetFailureKind,
    PinnedHttpsClient, PinnedHttpsClientConfiguration, SystemDnsResolver,
};
