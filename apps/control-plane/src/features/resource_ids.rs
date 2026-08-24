use uuid::{Uuid, Version};

pub(crate) fn parse_uuid_v7(value: &str) -> Result<Uuid, ()> {
    let value = Uuid::parse_str(value).map_err(|_| ())?;
    if value.get_version() == Some(Version::SortRand) {
        Ok(value)
    } else {
        Err(())
    }
}
