macro_rules! impl_display_fromstr {
    ($enum_name: ident { $($variant: ident),* }) => {
        #[derive(Debug, PartialEq)]
        pub enum $enum_name {
            $($variant),*
        }

        impl ::core::fmt::Display for $enum_name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                let s = match self {
                    $( $enum_name::$variant => stringify!($variant), )*
                };
                f.write_str(s)
            }
        }

        impl ::std::str::FromStr for $enum_name {
            type Err = &'static str;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $( stringify!($variant) => Ok($enum_name::$variant), )*
                    _ => Err(concat!("Invalid ", stringify!($enum_name), " string")),
                }
            }
        }
    };
}

impl_display_fromstr!(UserState {
    Active,
    Inactive,
    Expired
});

impl_display_fromstr!(SetState {
    SyncFile,
    NotSyncFile,
    Expired
});

impl_display_fromstr!(SetMethod {
    Pull,
    Push,
    Inactive
});
