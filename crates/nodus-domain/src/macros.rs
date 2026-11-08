/// Declares an enum of the strings a TypeScript union type allows. Each
/// variant is written as its string, `ALL` lists them in the TypeScript's
/// order (the `as const` arrays, such as FOLDER_COLORS, are these), and
/// `parse` is the TypeScript's `isX` guard.
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $(
                $(#[$variant_meta:meta])*
                $variant:ident = $value:literal
            ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        $vis enum $name {
            $(
                $(#[$variant_meta])*
                #[serde(rename = $value)]
                $variant,
            )+
        }

        impl $name {
            /// Every value, in the order the TypeScript lists them.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// The string the TypeScript uses for it.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value,)+
                }
            }

            /// The value a string names, if it names one.
            pub fn parse(value: &str) -> Option<Self> {
                match value {
                    $($value => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}
