/// State transfer messages don't really need internal verifications, since the entire message is signed
/// and the signature is verified by the network layer (by verifying the entire image)
pub trait StateTransferVerificationHelper {}
