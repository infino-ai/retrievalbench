// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! Entry point for the supertable-scale negation three-way.
//! See `negation_supertable.rs` for what is measured.

mod negation_supertable;

fn main() {
    negation_supertable::run();
}
