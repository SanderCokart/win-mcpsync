//! Commands shared by `envsync` and `mcpsync secret`.

use crate::prompt::read_secret;
use crate::ui::{die, dim, info, ok};
use crate::Store;

pub fn set_secret(store: &Store, n: &str) {
    let v = read_secret(&format!("Value for {n} (hidden): "))
        .unwrap_or_else(|e| die(&format!("can't read the value: {e}")));
    if v.is_empty() {
        die("empty value; nothing was set");
    }
    store.set(n, &v).unwrap_or_else(|e| die(&e));
    ok(&format!("{n} saved to your Windows user environment and shared with WSL (WSLENV)"));
    info(&dim("Reopen terminals and editors so they see it. In WSL: 'wsl --shutdown' first."));
}

pub fn unset_secret(store: &Store, n: &str) {
    store.unset(n).unwrap_or_else(|e| die(&e));
    ok(&format!("{n} removed from your Windows user environment and WSLENV"));
}
