# ⚠️ fav v0.* is achieved

`fav v0.*`, based on `fav_core` `fav_utils` (which heavily depends on **protobuf** and many traits),
is considered over-designed by me.

As my being more familar with Rust, I decide to re-factor again this CRUD-oriented application.

# 🆕 update

- **sqlite & sea-orm**: to support more video attributes management
- **better task manager**: a task manager with handle and callback
- **migrate tool**: help migrate from `fav v0.*` to `fav v1.*`
- **dep:api_req**: my published api request helper crate
- **dep:bevy_ecs**: make the app more maintainable
