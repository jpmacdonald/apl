//! UI Probe utility for testing table alignment.
#![allow(missing_docs)]

use apl_cli::ui::buffer::OutputBuffer;
use apl_cli::ui::list::{print_list_header, print_list_row, print_list_footer};

fn main() {
    let mut buffer = OutputBuffer::default();

    print_list_header(&mut buffer);
    print_list_row(&mut buffer, "bat", "0.26.1", 5_000_000, "2025-12-31");
    print_list_row(&mut buffer, "jq", "1.8.1", 0, "2025-12-31");
    print_list_row(&mut buffer, "just", "1.45.0", 4_000_000, "2025-12-31");
    print_list_footer(&mut buffer, 3, 9_000_000);

    buffer.flush();
}
