/// Catalogs expected to translate every message in the template.
///
/// The others are filled in as translators reach them: an empty `msgstr`
/// compiles out and shows the English source (fastframe-i18n tests that). A
/// catalog listed here must stay complete, and every catalog must keep the
/// placeholders of whatever it does translate.
const COMPLETE: &[&str] = &["ja"];

#[test]
fn catalogs_cover_the_template_and_preserve_named_placeholders() {
    // A POT leaves these values for msginit. For this comparison its source
    // language is English; the translator's PO carries its own actual rules.
    let template = include_str!("../assets/i18n/melba.pot").replace(
        "nplurals=INTEGER; plural=EXPRESSION;",
        "nplurals=2; plural=(n != 1);",
    );
    let template = polib::po_file::parse_from_reader(template.as_bytes()).unwrap();
    assert!(
        template
            .find_message(None, "melba cannot open its window", None)
            .is_some(),
        "the window-failure title must remain in the translation template"
    );
    let mut catalogs = 0;
    for file in std::fs::read_dir("assets/i18n").unwrap() {
        let path = file.unwrap().path();
        if path.extension().is_none_or(|ext| ext != "po") {
            continue;
        }
        catalogs += 1;
        let complete = path
            .file_stem()
            .is_some_and(|stem| COMPLETE.contains(&&*stem.to_string_lossy()));
        let translated_catalog = polib::po_file::parse(&path).unwrap();
        // Only non-empty msgids are messages: the file header is a header.
        // Comparing raw counts made the result depend on how the PO library
        // happens to classify that header, which is not what this test is for.
        //
        // A wrapped entry (`msgid ""` plus continuation lines) can arrive here
        // as an extra empty msgid from `msgmerge`; that is a broken catalog,
        // so it is rejected instead of quietly dropped from the comparison.
        // Only non-empty msgids are messages; the file header is not one. The
        // PO library classifies a header differently in a template and in a
        // catalog, so comparing raw counts made this test depend on that
        // classification instead of on catalogue coverage. Measured
        // 2026-10-28: `messages()` yields the catalog's header and not the
        // template's, which read as one missing message for every catalog.
        let messages = |catalog: &polib::catalog::Catalog| -> std::collections::BTreeSet<String> {
            catalog
                .messages()
                .map(|message| message.msgid())
                .filter(|msgid| !msgid.is_empty())
                .map(str::to_string)
                .collect()
        };
        let template_ids = messages(&template);
        let catalog_ids = messages(&translated_catalog);
        // Sets first: they name the offending message, which a count cannot.
        for missing in template_ids.difference(&catalog_ids) {
            panic!("{}: no entry for {:?}", path.display(), missing);
        }
        for extra in catalog_ids.difference(&template_ids) {
            panic!("{}: entry not in the template: {:?}", path.display(), extra);
        }
        assert_eq!(template_ids.len(), catalog_ids.len(), "{}", path.display());
        for source in template.messages() {
            let translated = translated_catalog
                .find_message(source.msgctxt(), source.msgid(), source.msgid_plural().ok())
                .unwrap_or_else(|| panic!("missing template entry for {}", source.msgid()));
            assert!(
                !translated.is_fuzzy(),
                "{}: {}",
                path.display(),
                source.msgid()
            );
            if let Ok(forms) = translated.msgstr_plural() {
                assert_eq!(
                    forms.len(),
                    translated_catalog.metadata.plural_rules.nplurals
                );
                for form in forms {
                    assert!(
                        !complete || !form.is_empty(),
                        "{} claims to be complete but {:?} is untranslated",
                        path.display(),
                        source.msgid()
                    );
                    if !form.is_empty() {
                        assert_eq!(named_placeholders(form), named_placeholders(source.msgid()));
                    }
                }
            } else {
                let text = translated.msgstr().unwrap();
                assert!(
                    !complete || !text.is_empty(),
                    "{} claims to be complete but {:?} is untranslated",
                    path.display(),
                    source.msgid()
                );
                if text.is_empty() {
                    continue;
                }
                assert_eq!(
                    named_placeholders(text),
                    named_placeholders(source.msgid()),
                    "{}: {}",
                    path.display(),
                    source.msgid()
                );
            }
        }
    }
    assert_eq!(
        catalogs + 1,
        <melba::i18n::Locale as clap::ValueEnum>::value_variants().len()
    );
}

fn named_placeholders(text: &str) -> Vec<&str> {
    let mut names: Vec<_> = text
        .split('{')
        .skip(1)
        .filter_map(|rest| rest.split_once('}').map(|(name, _)| name))
        .collect();
    names.sort_unstable();
    names
}
