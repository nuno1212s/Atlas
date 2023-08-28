use std::marker::PhantomData;

pub struct SigVerifier<SV, NI, D, OP, ST, LT>(PhantomData<(SV, NI, D, OP, LT, ST)>);