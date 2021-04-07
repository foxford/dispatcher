use anyhow::Result;

use super::{RtcUploadReadyData, RtcUploadResult};

pub(super) fn extract_ready_rtcs(rtcs: Vec<RtcUploadResult>) -> Result<Vec<RtcUploadReadyData>> {
    let mut ready_rtcs = Vec::with_capacity(rtcs.len());

    for rtc in rtcs {
        match rtc {
            RtcUploadResult::Ready(ready_rtc) => ready_rtcs.push(ready_rtc),
            RtcUploadResult::Missing(missing_rtc) => {
                bail!("RTC {} is missing recording", missing_rtc.id)
            }
        }
    }

    Ok(ready_rtcs)
}
