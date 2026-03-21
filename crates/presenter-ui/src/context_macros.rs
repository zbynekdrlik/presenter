/// Helper to extract Leptos context with a clear panic message including the type name.
/// Panics in WASM become browser-side JavaScript errors (not server crashes).
macro_rules! use_ctx {
    ($t:ty) => {
        use_context::<$t>().expect(concat!(
            "Missing context: ",
            stringify!($t),
            " — ensure provider is mounted"
        ))
    };
}
pub(crate) use use_ctx;
