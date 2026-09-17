mod write_only {
    pub struct Surface {
        pub id: u32,
    }
}

structio::object!(write_only::Surface { id });

fn main() {}
