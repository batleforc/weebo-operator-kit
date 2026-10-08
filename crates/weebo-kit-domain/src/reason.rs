//! Stable reason codes. Each operator declares its own catalog with
//! [`reason_codes!`](crate::reason_codes) and implements [`Reason`] on it:
//! status conditions, Events, metrics and admission denials only ever carry
//! one of its variants, never an ad-hoc string.

/// Whether a reason blocks `Ready` or is purely advisory (the object still
/// works; the condition is a stable, discoverable signal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    Blocking,
    Advisory,
}

/// An operator's reason-code catalog, as the kit's generic code sees it.
pub trait Reason: Copy + Eq + std::fmt::Debug + Send + Sync + 'static {
    /// The reason of `Ready: True`.
    const SUCCESS: Self;
    /// PascalCase, valid as a `Condition`/`Event` `reason`.
    fn as_str(self) -> &'static str;
    fn severity(self) -> Severity;
}

/// Declares a reason-code enum, its `as_str()`, an `ALL` slice and
/// `Display` from one list, so a variant can't be added without its string
/// form. `Reason` is then implemented by hand (its success variant and
/// which codes are advisory are the operator's call).
///
/// ```
/// weebo_kit_domain::reason_codes! {
///     pub enum ReasonCode {
///         /// `Ready: True`.
///         Reconciled,
///         InvalidSpec,
///     }
/// }
/// assert_eq!(ReasonCode::InvalidSpec.as_str(), "InvalidSpec");
/// assert_eq!(ReasonCode::ALL.len(), 2);
/// ```
#[macro_export]
macro_rules! reason_codes {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $($(#[$doc:meta])* $variant:ident),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        $vis enum $name {
            $($(#[$doc])* $variant,)+
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant,)+];

            /// PascalCase representation, valid as both a Kubernetes
            /// `Condition`/`Event` `reason` and an admission response
            /// message prefix.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => stringify!($variant),)+
                }
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

/// Every code is PascalCase with no separator — usable as a condition
/// `type`/`reason`. For an operator's own catalog test.
pub fn assert_pascal_case<R: Reason>(all: &[R]) {
    for code in all {
        let s = code.as_str();
        assert!(
            s.chars().next().is_some_and(|c| c.is_ascii_uppercase()),
            "{s}"
        );
        assert!(s.chars().all(|c| c.is_ascii_alphanumeric()), "{s}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::reason_codes! {
        enum Code {
            Reconciled,
            Pending,
            Broken,
        }
    }

    impl Reason for Code {
        const SUCCESS: Self = Code::Reconciled;
        fn as_str(self) -> &'static str {
            Code::as_str(self)
        }
        fn severity(self) -> Severity {
            match self {
                Code::Reconciled | Code::Pending => Severity::Advisory,
                Code::Broken => Severity::Blocking,
            }
        }
    }

    #[test]
    fn the_macro_derives_names_and_the_full_list() {
        assert_eq!(Code::ALL, &[Code::Reconciled, Code::Pending, Code::Broken]);
        assert_eq!(Code::Broken.to_string(), "Broken");
        assert_pascal_case(Code::ALL);
        assert_eq!(Code::SUCCESS.severity(), Severity::Advisory);
    }
}
