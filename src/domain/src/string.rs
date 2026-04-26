macro_rules! define_string {
    ($name:ident, max = $max:expr) => {
        $crate::string::define_string!(@build $name, min = 1, max = $max);
    };
    ($name:ident, min = $min:expr, max = $max:expr) => {
        $crate::string::define_string!(@build $name, min = $min, max = $max);
    };
    (@build $name:ident, min = $min:expr, max = $max:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name {
            value: String,
            _hide_default_constructor: ::core::marker::PhantomData<()>,
        }

        impl $name {
            pub fn value(&self) -> &str {
                &self.value
            }
        }

        impl ::core::convert::TryFrom<String> for $name {
            type Error = $crate::error::Error;
            fn try_from(value: String) -> ::core::result::Result<Self, Self::Error> {
                let len = value.chars().count();
                if !($min..=$max).contains(&len) {
                    return Err($crate::error::Error::InvalidLengthRange(format!(
                        "{} must have length in {}..={}, got {}",
                        stringify!($name),
                        $min,
                        $max,
                        len
                    )));
                }
                if value.chars().any(|c| c.is_control()) {
                    return Err($crate::error::Error::InvalidFormat(format!(
                        "{} contains control characters",
                        stringify!($name),
                    )));
                }
                Ok(Self {
                    value,
                    _hide_default_constructor: ::core::marker::PhantomData,
                })
            }
        }

        impl ::core::convert::TryFrom<&str> for $name {
            type Error = $crate::error::Error;
            fn try_from(value: &str) -> ::core::result::Result<Self, Self::Error> {
                <$name as ::core::convert::TryFrom<String>>::try_from(value.to_string())
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(&self.value)
            }
        }
    };
}

pub(crate) use define_string;
