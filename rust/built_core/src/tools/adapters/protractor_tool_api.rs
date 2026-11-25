// SPDX-License-Identifier: GPL-3.0-or-later
//! ProtractorTool callbacks on the live document.
use super::*;

fn set_arm<'py>(a: &Args<'py>) -> Reply<'py> {
    let py = a.py();
    let module = imported(py, "tools.rust_tools")?;
    let key = a.get_item(1)?;
    let p = a.get_item(2)?;
    let tool = a.get_item(0)?;
    if tool.getattr("_during")?.is_none() {
        tool.getattr("_machine")?
            .call_method1("set", (&key, module.call_method1("_tuple", (&p,))?))?;
    }
    Ok(none(py))
}
#[rustfmt::skip]
pub(super) const METHODS: &[Method] = &[
    ("_set_arm", Mode::Method, set_arm, &["self", "key", "p"], "", ""),
];
