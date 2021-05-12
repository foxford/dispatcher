use chrono::{DateTime, SubsecRound, TimeZone};

pub fn random_string() -> String {
    use rand::distributions::Alphanumeric;
    use rand::{thread_rng, Rng};

    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(30)
        .map(char::from)
        .collect()
}

// datetimes coming from db sometimes lose precision (on a scale of nanoseconds), like this:
//   left: `2021-05-12T08:57:14.322719Z`,
//  right: `2021-05-12T08:57:14.322719819Z`'
// equality up to ms should be enough
pub fn datetimes_almost_eq<Tz: TimeZone>(dt1: DateTime<Tz>, dt2: DateTime<Tz>) -> bool {
    dt1.trunc_subsecs(3) == dt2.trunc_subsecs(3)
}
