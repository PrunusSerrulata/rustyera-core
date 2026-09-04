mod configuration;
mod extensions;
mod frontend;
mod model;
mod support;
#[cfg(test)]
mod tests;

include!("project/facade.rs");
include!("project/build.rs");
include!("project/delta.rs");
