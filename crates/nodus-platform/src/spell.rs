//! The OS spellchecker (spec 01 PLAT-137): Chromium underlined misspelled
//! words in the text fields that didn't opt out, with the macOS system
//! spellchecker (Hunspell on Windows). The port asks the OS's own: AppKit's
//! `NSSpellChecker` on a Mac, `ISpellChecker` on Windows (plan.md,
//! "Decisions for the user", spellcheck).

use std::ops::Range;

/// The misspelled words of `text`, as UTF-16 ranges, first to last. None
/// when the OS has no spellchecker to ask.
pub fn misspelled(text: &str) -> Vec<Range<usize>> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    #[cfg(target_os = "macos")]
    {
        mac::misspelled(text)
    }
    #[cfg(windows)]
    {
        windows::misspelled(text)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ops::Range;

    use objc2_app_kit::NSSpellChecker;
    use objc2_foundation::{NSNotFound, NSString};

    pub(super) fn misspelled(text: &str) -> Vec<Range<usize>> {
        let checker = NSSpellChecker::sharedSpellChecker();
        let string = NSString::from_str(text);
        let len = string.length();
        let mut found = Vec::new();
        let mut at = 0usize;
        while at < len {
            // SAFETY: a null word count is allowed; language nil is the
            // user's, and tag 0 is the shared document.
            let range = unsafe {
                checker.checkSpellingOfString_startingAt_language_wrap_inSpellDocumentWithTag_wordCount(
                    &string,
                    at as isize,
                    None,
                    false,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if range.location == NSNotFound as usize || range.length == 0 || range.location < at {
                break;
            }
            found.push(range.location..range.location + range.length);
            at = range.location + range.length;
        }
        found
    }
}

#[cfg(windows)]
mod windows {
    use std::ops::Range;
    use std::sync::OnceLock;

    use windows::Win32::Globalization::{ISpellChecker, ISpellCheckerFactory, SpellCheckerFactory};
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
    use windows::core::{HSTRING, PCWSTR};

    /// The user's language's checker, made once on a COM thread of its own.
    struct Checker(Option<ISpellChecker>);

    // SAFETY: made in the multithreaded apartment, so any thread may call it.
    unsafe impl Send for Checker {}
    // SAFETY: as above.
    unsafe impl Sync for Checker {}

    fn checker() -> Option<&'static ISpellChecker> {
        static CHECKER: OnceLock<Checker> = OnceLock::new();
        CHECKER
            .get_or_init(|| {
                // SAFETY: joins the multithreaded apartment (or finds this
                // thread already in one), then makes the factory and asks
                // it for the user's language.
                let made = unsafe {
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                    let factory: windows::core::Result<ISpellCheckerFactory> =
                        CoCreateInstance(&SpellCheckerFactory, None, CLSCTX_INPROC_SERVER);
                    factory.ok().and_then(|f| {
                        let language = HSTRING::from(user_language());
                        let supported = f.IsSupported(&language).is_ok_and(windows::core::BOOL::as_bool);
                        let language = if supported { language } else { HSTRING::from("en-US") };
                        f.CreateSpellChecker(&language).ok()
                    })
                };
                Checker(made)
            })
            .0
            .as_ref()
    }

    fn user_language() -> String {
        use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
        let mut name = [0u16; 85];
        // SAFETY: the buffer and its length match.
        let len = unsafe { GetUserDefaultLocaleName(name.as_mut_ptr(), name.len() as i32) };
        if len > 1 {
            String::from_utf16_lossy(&name[..len as usize - 1])
        } else {
            "en-US".into()
        }
    }

    pub(super) fn misspelled(text: &str) -> Vec<Range<usize>> {
        let Some(checker) = checker() else {
            return Vec::new();
        };
        let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let mut found = Vec::new();
        // SAFETY: the text is NUL-terminated and outlives the call; the
        // errors are read until the enumerator says there are no more.
        unsafe {
            let Ok(errors) = checker.Check(PCWSTR(wide.as_ptr())) else {
                return found;
            };
            loop {
                let mut error = None;
                if errors.Next(&mut error).is_err() {
                    break;
                }
                let Some(error) = error else { break };
                if let (Ok(start), Ok(len)) = (error.StartIndex(), error.Length()) {
                    found.push(start as usize..(start + len) as usize);
                }
            }
        }
        found
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn finds_the_words_the_system_doesnt_know() {
        let text = "Deploy the storefrnt to stagign";
        let found = misspelled(text);
        if found.is_empty() {
            // No spell server here (a headless runner): nothing to check.
            return;
        }
        let words: Vec<&str> = found.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(words, ["storefrnt", "stagign"]);
        assert!(misspelled("Deploy the storefront").is_empty());
        assert!(misspelled("   ").is_empty());
    }
}
