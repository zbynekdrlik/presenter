use std::path::PathBuf;

use presenter_importer::proto;
use prost::Message;

fn main() -> anyhow::Result<()> {
    let path = PathBuf::from(std::env::args().nth(1).expect("path"));
    let bytes = std::fs::read(&path)?;
    let raw = proto::Presentation::decode(&*bytes)?;
    println!("arrangements: {}", raw.arrangements.len());
    for (idx, arrangement) in raw.arrangements.iter().enumerate() {
        println!("arrangement {} name {:?}", idx, arrangement.name);
        for group_id in &arrangement.group_identifiers {
            println!("  group id {:?}", group_id.string);
        }
    }
    println!("cue_groups: {}", raw.cue_groups.len());
    for (idx, cue_group) in raw.cue_groups.iter().enumerate() {
        let group_uuid = cue_group
            .group
            .as_ref()
            .and_then(|g| g.uuid.as_ref())
            .map(|u| u.string.clone());
        println!(
            "cue_group {} uuid {:?} name {:?}",
            idx,
            group_uuid,
            cue_group.group.as_ref().map(|g| g.name.clone())
        );
        for cue_id in &cue_group.cue_identifiers {
            println!("  cue uuid {}", cue_id.string);
        }
    }
    Ok(())
}
