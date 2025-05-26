This utils is an example for [`fav_core`](https://crates.io/crates/fav_core).

# `fav_core`
`fav_core` provides the core traits needed to create an app, which fetch your remote favorite sets(lists), and pull tracked resources to local.

# `fav_utils`
[`fav_utils`](https://crates.io/crates/fav_utils) provides the utilities for [`fav_cli`](https://crates.io/crates/fav_cli), which now only support [bilibili](https://www.bilibili.com)(Like Chinese Youtube).

I really want to support more websites, and make utils and cli more generic to use, but I don't have enough time to do that. So if you like, you can make use of `fav_core` to create your own app!!!

All you need is to define data structures with [`protobuf`](https://protobuf.dev) like [this example](https://github.com/kingwingfly/fav/blob/dev/fav_utils/proto/bili.proto).

And impl traits in `fav_core` like `Res`, `Set`, `Sets`, `Status`, `Attr` to defined resources, and `AuthOps`, `SetOps`, `SetsOps` to operate resources.

After that, many `Ext` method will be available for you to use, helping you **batchly** operating resource, getting subset of resource, and so on.

Examples can be find [here](https://github.com/kingwingfly/fav)
