use super::HealthArgs;

pub fn healthcheck(args: HealthArgs) -> anyhow::Result<i32> {
    Ok(if crate::health::is_ready(&args.status_file) {
        0
    } else {
        1
    })
}
