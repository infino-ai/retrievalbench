// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Infino Authors

//! Entry point for the single-segment negation three-way.
//! See `negation.rs` for what is measured and verified.

mod negation;

fn main() {
    negation::run();
}
