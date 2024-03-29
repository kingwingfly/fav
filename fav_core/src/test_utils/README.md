This is used for doc tests.

Some traits in this crate need `protobuf::MessageFull`, such as `ProtoLocal: PathInfo + MessageFull`, which is auto implemented for all `T: PathInfo + MessageFull`.

To write `ProtoLocal`'s doc test, we have to got a struct (let's name it `Msg`) that satisfies `MessageFull` and `PathInfo` traits.

However, it is too hard to mamually implemente `MessageFull`. (`MessageFull` is generated by the famous [`protobuf`](https://protobuf.dev), the basis of kinds of `RPC`s). If we generate and `pub use` it in `my_crate`, we cannot `impl PathInfo for Msg` due to rust's orphan rule (We all know that document test is a kind of integration test).

```rust
// in doc test
use my_crate::test_utils::Msg;
// `Msg` is in `my_crate`'s test_utils, which implemented `MessageFull`
use my_crate::{PathInfo, ProtoLocal};

impl PathInfo for Mag { ... } // <- this violate rust's orphan rule
```

Instead we can:
```rust
// in doc test
// Included through `mod`.
mod test_utils;
use test_utils::msg::Msg;
use my_crate::{PathInfo, ProtoLocal};

impl PathInfo for Mag { ... }
// Here `Msg` is part of the doc test, just like `common` introduced in offical book about integration tests,
// and won't violate rust's orphan rule
# fn main() {}    // <- This is essential, or an error will be raised
```

Caution: One has to add `# fn main() {}` at the end of the test, or an error will raised.

Or, one should use `#[path = "..."]` to manually refer to the file as said [here](https://doc.rust-lang.org/reference/items/modules.html) if without `main()`.

# Related
- [document](https://doc.rust-lang.org/book/ch11-03-test-organization.html#submodules-in-integration-tests)
- [module-system](https://aloso.github.io/2021/03/28/module-system.html)
