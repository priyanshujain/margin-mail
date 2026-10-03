use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProofIssue {
    start: usize,
    end: usize,
    kind: &'static str,
    message: String,
    suggestions: Vec<String>,
}

#[tauri::command]
pub async fn proof_text(text: String, spelling: bool, grammar: bool) -> Result<Vec<ProofIssue>, String> {
    #[cfg(target_os = "macos")]
    {
        tauri::async_runtime::spawn_blocking(move || check(&text, spelling, grammar))
            .await
            .map_err(|error| error.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (text, spelling, grammar);
        Ok(Vec::new())
    }
}

#[cfg(target_os = "macos")]
fn check(text: &str, spelling: bool, grammar: bool) -> Vec<ProofIssue> {
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::NSSpellChecker;
    use objc2_foundation::{NSArray, NSRange, NSString, NSTextCheckingType, NSValue};

    autoreleasepool(|_| {
        let checker = NSSpellChecker::sharedSpellChecker();
        let string = NSString::from_str(text);
        let mut types = NSTextCheckingType::empty();
        if spelling {
            types |= NSTextCheckingType::Spelling;
        }
        if grammar {
            types |= NSTextCheckingType::Grammar;
        }
        let results = unsafe {
            checker.checkString_range_types_options_inSpellDocumentWithTag_orthography_wordCount(
                &string,
                NSRange { location: 0, length: string.length() },
                types.bits(),
                None,
                0,
                None,
                std::ptr::null_mut(),
            )
        };
        let mut issues = Vec::new();
        for result in results.iter() {
            let range = result.range();
            if result.resultType() == NSTextCheckingType::Spelling {
                let suggestions = checker
                    .guessesForWordRange_inString_language_inSpellDocumentWithTag(range, &string, None, 0)
                    .map(|guesses| guesses.iter().take(5).map(|guess| guess.to_string()).collect())
                    .unwrap_or_default();
                issues.push(ProofIssue {
                    start: range.location,
                    end: range.location + range.length,
                    kind: "spelling",
                    message: "Check spelling".into(),
                    suggestions,
                });
            } else if result.resultType() == NSTextCheckingType::Grammar {
                if let Some(details) = result.grammarDetails() {
                    for detail in details.iter() {
                        let relative = detail.objectForKey(&NSString::from_str("NSGrammarRange"))
                            .and_then(|value| value.downcast_ref::<NSValue>().map(|value| unsafe { value.rangeValue() }));
                        let start = range.location + relative.map_or(0, |value| value.location);
                        let length = relative.map_or(range.length, |value| value.length);
                        let message = detail.objectForKey(&NSString::from_str("NSGrammarUserDescription"))
                            .and_then(|value| value.downcast_ref::<NSString>().map(|value| value.to_string()))
                            .unwrap_or_else(|| "Check grammar".into());
                        let suggestions = detail.objectForKey(&NSString::from_str("NSGrammarCorrections"))
                            .and_then(|value| value.downcast_ref::<NSArray>().map(|values| values.iter().filter_map(|value| value.downcast_ref::<NSString>().map(|value| value.to_string())).take(5).collect()))
                            .unwrap_or_default();
                        issues.push(ProofIssue { start, end: start + length, kind: "grammar", message, suggestions });
                    }
                }
            }
        }
        issues
    })
}
