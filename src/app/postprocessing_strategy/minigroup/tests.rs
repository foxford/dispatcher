mod handle_upload {
    use std::ops::Bound;
    use std::sync::Arc;

    use chrono::{DateTime, Duration, Utc};
    use uuid::Uuid;

    use crate::app::{postprocessing_strategy::StreamData, AppContext};
    use crate::clients::event::test_helpers::EventBuilder;
    use crate::clients::event::{EventData, HostEventData};
    use crate::db::recording::{RecordingListQuery, Segments};
    use crate::test_helpers::{prelude::*, shared_helpers::random_string};

    use super::super::super::PostprocessingStrategy;
    use super::super::*;

    #[tokio::test]
    async fn handle_upload_stream() {
        let now = Utc::now();
        let mut state = TestState::new(TestAuthz::new()).await;
        let conference_room_id = Uuid::new_v4();
        let event_room_id = Uuid::new_v4();
        let agent1 = TestAgent::new("web", "user1", USR_AUDIENCE);
        let agent1_clone = agent1.clone();
        let agent2 = TestAgent::new("web", "user2", USR_AUDIENCE);
        let agent3 = TestAgent::new("web", "user3", USR_AUDIENCE);

        // Insert a minigroup.
        let minigroup = {
            let mut conn = state.get_conn().await.expect("Failed to get conn");

            let time = (
                Bound::Included(now - Duration::hours(1)),
                Bound::Excluded(now - Duration::minutes(10)),
            );

            let minigroup_scope = format!("minigroup-{}", random_string());

            factory::Minigroup::new(
                minigroup_scope,
                USR_AUDIENCE.to_string(),
                time.into(),
                conference_room_id,
                event_room_id,
            )
            .insert(&mut conn)
            .await
        };
        let rtc1_id = Uuid::new_v4();
        let rtc2_id = Uuid::new_v4();
        let rtc3_id = Uuid::new_v4();

        let minigroup_id = minigroup.id();

        {
            let mut conn = state.get_conn().await.expect("Failed to get conn");
            let agent1 = TestAgent::new("web", "user1", USR_AUDIENCE);

            factory::Recording::new(minigroup_id, rtc1_id, agent1.agent_id().clone())
                .insert(&mut conn)
                .await;

            let agent2 = TestAgent::new("web", "user2", USR_AUDIENCE);
            factory::Recording::new(minigroup_id, rtc2_id, agent2.agent_id().clone())
                .insert(&mut conn)
                .await;

            factory::Recording::new(minigroup_id, rtc3_id, agent3.agent_id().clone())
                .insert(&mut conn)
                .await;
        };

        // Set up event client mock.
        let started_at1: DateTime<Utc> = now - Duration::hours(1);

        let segments1: Segments = vec![
            (Bound::Included(0), Bound::Excluded(1500000)),
            (Bound::Included(1800000), Bound::Excluded(3000000)),
        ]
        .into();

        let expected_segments = segments1.clone();

        state
            .event_client_mock()
            .expect_list_events()
            .withf(move |room_id: &Uuid, _kind: &str| {
                assert_eq!(*room_id, event_room_id);
                true
            })
            .returning(move |_, kind| match kind {
                HOST_EVENT_TYPE => Ok(vec![EventBuilder::new()
                    .room_id(event_room_id)
                    .set(HOST_EVENT_TYPE.to_string())
                    .data(EventData::Host(HostEventData::new(
                        agent1_clone.agent_id().to_owned(),
                    )))
                    .occurred_at(0)
                    .build()]),
                other => panic!("Event client mock got unknown kind: {}", other),
            });

        state.set_audience_preroll_offset(minigroup.audience(), 1234);

        state
            .event_client_mock()
            .expect_adjust_room()
            .withf(
                move |room_id: &Uuid,
                      started_at: &DateTime<Utc>,
                      segments: &Segments,
                      offset: &i64| {
                    assert_eq!(*room_id, event_room_id);
                    assert_eq!(
                        started_at.timestamp_millis(),
                        started_at1.timestamp_millis()
                    );
                    assert_eq!(segments, &expected_segments);
                    assert_eq!(*offset, 1234);
                    true
                },
            )
            .returning(|_, _, _, _| Ok(()));

        // Handle uploading two RTCs.
        let uri1 = "s3://minigroup.origin.dev.example.com/rtc1.webm";

        let stream1 = UploadedStream {
            id: rtc1_id,
            parsed_data: Ok(StreamData {
                uri: uri1.to_string(),
                started_at: started_at1,
                segments: segments1.clone(),
            }),
        };

        let uri2 = "s3://minigroup.origin.dev.example.com/rtc2.webm";
        let started_at2 = now - Duration::minutes(50);
        let segments2: Segments = vec![(Bound::Included(0), Bound::Excluded(2700000))].into();

        let stream2 = UploadedStream {
            id: rtc2_id,
            parsed_data: Ok(StreamData {
                uri: uri2.to_string(),
                started_at: started_at2,
                segments: segments2.clone(),
            }),
        };

        let stream3 = UploadedStream {
            id: rtc3_id,
            parsed_data: Err(anyhow!("Err")),
        };

        let state = Arc::new(state);

        MinigroupPostprocessingStrategy::new(state.clone(), minigroup.clone())
            .handle_stream_upload(stream1)
            .await
            .expect("Failed to handle upload");

        // Assert recordings in the DB.
        {
            let mut conn = state.get_conn().await.expect("Failed to get conn");

            let ready_items = RecordingListQuery::new(minigroup_id)
                .execute(&mut conn)
                .await
                .expect("Failed to list recordings")
                .into_iter()
                .filter_map(|recording| ReadyRecording::from_db_object(&recording))
                .count();
            assert_eq!(ready_items, 1);
        }

        MinigroupPostprocessingStrategy::new(state.clone(), minigroup.clone())
            .handle_stream_upload(stream2)
            .await
            .expect("Failed to handle upload");

        let recordings = {
            let mut conn = state.get_conn().await.expect("Failed to get conn");

            RecordingListQuery::new(minigroup_id)
                .execute(&mut conn)
                .await
                .expect("Failed to list recordings")
                .into_iter()
                .filter_map(|recording| ReadyRecording::from_db_object(&recording))
                .collect::<Vec<_>>()
        };

        MinigroupPostprocessingStrategy::new(state.clone(), minigroup)
            .handle_stream_upload(stream3)
            .await
            .expect("Failed to handle upload");

        assert_eq!(recordings.len(), 2);

        let recording1 = recordings
            .iter()
            .find(|recording| recording.rtc_id == rtc1_id)
            .expect("Recording 1 not found");

        assert_eq!(&recording1.stream_uri, uri1);
        assert!(datetimes_almost_eq(recording1.started_at, started_at1));
        assert_eq!(&recording1.segments, &segments1);
        assert_eq!(&recording1.created_by, agent1.agent_id());

        let recording2 = recordings
            .iter()
            .find(|recording| recording.rtc_id == rtc2_id)
            .expect("Recording 2 not found");

        assert_eq!(&recording2.stream_uri, uri2);
        assert!(datetimes_almost_eq(
            recording2.started_at,
            now - Duration::minutes(50)
        ));
        assert_eq!(&recording2.segments, &segments2);
        assert_eq!(&recording2.created_by, agent2.agent_id());
    }

    #[tokio::test]
    async fn handle_upload_mjr() {
        let now = Utc::now();
        let mut state = TestState::new(TestAuthz::new()).await;
        let conference_room_id = Uuid::new_v4();
        let event_room_id = Uuid::new_v4();

        // Insert a minigroup.
        let minigroup = {
            let mut conn = state.get_conn().await.expect("Failed to get conn");

            let time = (
                Bound::Included(now - Duration::hours(1)),
                Bound::Excluded(now - Duration::minutes(10)),
            );

            let minigroup_scope = format!("minigroup-{}", random_string());

            factory::Minigroup::new(
                minigroup_scope,
                USR_AUDIENCE.to_string(),
                time.into(),
                conference_room_id,
                event_room_id,
            )
            .insert(&mut conn)
            .await
        };

        let minigroup_id = minigroup.id();

        let dumps = vec![
            "s3://minigroup.origin.dev.example.com/rtc1.mjr".to_owned(),
            "s3://minigroup.origin.dev.example.com/rtc2.mjr".to_owned(),
        ];

        state
            .tq_client_mock()
            .expect_create_task()
            .times(2)
            .returning(|_, _, _| Ok(()));

        // Handle uploading two RTCs.
        let rtc1_id = Uuid::new_v4();
        let uri1 = "s3://minigroup.origin.dev.example.com/rtc1.webm";
        let agent1 = TestAgent::new("web", "user1", USR_AUDIENCE);

        let rtc1 = MjrDumpsUploadResult::Ready(MjrDumpsUploadReadyData {
            id: rtc1_id,
            uri: uri1.to_string(),
            created_by: agent1.agent_id().to_owned(),
            mjr_dumps_uris: dumps.clone(),
        });

        let rtc2_id = Uuid::new_v4();
        let uri2 = "s3://minigroup.origin.dev.example.com/rtc2.webm";
        let agent2 = TestAgent::new("web", "user2", USR_AUDIENCE);

        let rtc2 = MjrDumpsUploadResult::Ready(MjrDumpsUploadReadyData {
            id: rtc2_id,
            uri: uri2.to_string(),
            created_by: agent2.agent_id().to_owned(),
            mjr_dumps_uris: dumps.clone(),
        });

        let state = Arc::new(state);

        MinigroupPostprocessingStrategy::new(state.clone(), minigroup)
            .handle_mjr_dumps_upload(vec![rtc1, rtc2])
            .await
            .expect("Failed to handle upload");

        // Assert recordings in the DB.
        let recordings = {
            let mut conn = state.get_conn().await.expect("Failed to get conn");

            RecordingListQuery::new(minigroup_id)
                .execute(&mut conn)
                .await
                .expect("Failed to list recordings")
        };

        assert_eq!(recordings.len(), 2);

        let recording1 = recordings
            .iter()
            .find(|recording| recording.rtc_id() == rtc1_id)
            .expect("Recording 1 not found");

        assert_eq!(recording1.stream_uri(), None);
        assert_eq!(recording1.segments(), None);
        assert_eq!(recording1.created_by(), agent1.agent_id());

        let recording2 = recordings
            .iter()
            .find(|recording| recording.rtc_id() == rtc2_id)
            .expect("Recording 2 not found");

        assert_eq!(recording2.stream_uri(), None);
        assert_eq!(recording2.segments(), None);
        assert_eq!(recording2.created_by(), agent2.agent_id());
    }
}

// mod handle_adjust {
//     use std::ops::Bound;
//     use std::sync::Arc;

//     use chrono::{Duration, Utc};
//     use uuid::Uuid;

//     use crate::app::AppContext;
//     use crate::clients::event::test_helpers::EventBuilder;
//     use crate::clients::event::{EventData, EventRoomResponse, HostEventData, PinEventData};
//     use crate::db::class::MinigroupReadQuery;
//     use crate::db::recording::{RecordingListQuery, Segments};
//     use crate::test_helpers::{prelude::*, shared_helpers::random_string};

//     use super::super::super::PostprocessingStrategy;
//     use super::super::*;

//     #[tokio::test]
//     async fn handle_adjust() {
//         let now = Utc::now();
//         let agent1 = TestAgent::new("web", "user1", USR_AUDIENCE);
//         let agent2 = TestAgent::new("web", "user2", USR_AUDIENCE);
//         let mut state = TestState::new(TestAuthz::new()).await;
//         let event_room_id = Uuid::new_v4();
//         let original_event_room_id = Uuid::new_v4();
//         let modified_event_room_id = Uuid::new_v4();
//         let conference_room_id = Uuid::new_v4();

//         let original_host_segments: Segments = vec![
//             (Bound::Included(0), Bound::Excluded(1500000)),
//             (Bound::Included(1800000), Bound::Excluded(3000000)),
//         ]
//         .into();

//         // assume there was single cut-stop at the beginning
//         let modified_segments: Segments =
//             vec![(Bound::Included(0), Bound::Excluded(2_697_000))].into();

//         let cut_original_segments: Segments = vec![
//             (Bound::Included(3_000), Bound::Excluded(1_500_000)),
//             (Bound::Included(1_800_000), Bound::Excluded(3_000_000)),
//         ]
//         .into();

//         // Insert a minigroup with recordings.
//         let (minigroup, recording1, recording2) = {
//             let mut conn = state.get_conn().await.expect("Failed to get conn");

//             let time = (
//                 Bound::Included(now - Duration::hours(1)),
//                 Bound::Excluded(now - Duration::minutes(10)),
//             );

//             let minigroup_scope = format!("minigroup-{}", random_string());

//             let minigroup = factory::Minigroup::new(
//                 minigroup_scope,
//                 USR_AUDIENCE.to_string(),
//                 time.into(),
//                 conference_room_id,
//                 event_room_id,
//             )
//             .insert(&mut conn)
//             .await;

//             let recording1 = factory::Recording::new(
//                 minigroup.id(),
//                 Uuid::new_v4(),
//                 agent1.agent_id().to_owned(),
//             )
//             .stream_uri("s3://minigroup.origin.dev.example.com/rtc1.webm".to_string())
//             .segments(original_host_segments.clone())
//             .started_at(now - Duration::hours(1))
//             .insert(&mut conn)
//             .await;

//             let recording2 = factory::Recording::new(
//                 minigroup.id(),
//                 Uuid::new_v4(),
//                 agent2.agent_id().to_owned(),
//             )
//             .stream_uri("s3://minigroup.origin.dev.example.com/rtc2.webm".to_string())
//             .segments(vec![(Bound::Included(0), Bound::Excluded(2700000))].into())
//             .started_at(now - Duration::minutes(50))
//             .insert(&mut conn)
//             .await;

//             (minigroup, recording1, recording2)
//         };

//         let minigroup_id = minigroup.id();

//         state
//             .conference_client_mock()
//             .expect_read_config_snapshots()
//             .with(mockall::predicate::eq(conference_room_id))
//             .returning(move |_room_id| Ok(vec![]));

//         // Set up event client mock.
//         state
//             .event_client_mock()
//             .expect_read_room()
//             .with(mockall::predicate::eq(modified_event_room_id))
//             .returning(move |room_id| {
//                 Ok(EventRoomResponse {
//                     id: room_id,
//                     time: (
//                         Bound::Included(now - Duration::hours(1)),
//                         Bound::Excluded(now - Duration::minutes(10)),
//                     ),
//                     tags: None,
//                 })
//             });

//         state
//             .event_client_mock()
//             .expect_dump_room()
//             .with(mockall::predicate::eq(modified_event_room_id))
//             .returning(move |_room_id| Ok(()));

//         state
//             .event_client_mock()
//             .expect_list_events()
//             .withf(move |room_id: &Uuid, _kind: &str| {
//                 assert_eq!(*room_id, modified_event_room_id);
//                 true
//             })
//             .returning(move |_, kind| match kind {
//                 PIN_EVENT_TYPE => Ok(vec![
//                     EventBuilder::new()
//                         .room_id(modified_event_room_id)
//                         .set(PIN_EVENT_TYPE.to_string())
//                         .data(EventData::Pin(PinEventData::new(
//                             agent1.agent_id().to_owned(),
//                         )))
//                         .occurred_at(0)
//                         .build(),
//                     EventBuilder::new()
//                         .room_id(modified_event_room_id)
//                         .set(PIN_EVENT_TYPE.to_string())
//                         .data(EventData::Pin(PinEventData::new(
//                             agent2.agent_id().to_owned(),
//                         )))
//                         .occurred_at(1200000000000)
//                         .build(),
//                     EventBuilder::new()
//                         .room_id(modified_event_room_id)
//                         .set(PIN_EVENT_TYPE.to_string())
//                         .data(EventData::Pin(PinEventData::new(
//                             agent1.agent_id().to_owned(),
//                         )))
//                         .occurred_at(1500000000000)
//                         .build(),
//                 ]),
//                 HOST_EVENT_TYPE => Ok(vec![EventBuilder::new()
//                     .room_id(modified_event_room_id)
//                     .set(HOST_EVENT_TYPE.to_string())
//                     .data(EventData::Host(HostEventData::new(
//                         agent1.agent_id().to_owned(),
//                     )))
//                     .occurred_at(0)
//                     .build()]),
//                 other => panic!("Event client mock got unknown kind: {}", other),
//             });

//         // Set up tq client mock.
//         let uri1 = recording1.stream_uri().unwrap().clone();
//         let uri2 = recording2.stream_uri().unwrap().clone();

//         let expected_task = TqTask::TranscodeMinigroupToHls {
//             streams: vec![
//                 TranscodeMinigroupToHlsStream::new(recording1.rtc_id(), uri1)
//                     .offset(0)
//                     .segments(original_host_segments.clone())
//                     .modified_segments(cut_original_segments.clone())
//                     .pin_segments(
//                         vec![
//                             (Bound::Included(0), Bound::Excluded(1200000)),
//                             (Bound::Included(1500000), Bound::Excluded(3000000)),
//                         ]
//                         .into(),
//                     ),
//                 TranscodeMinigroupToHlsStream::new(recording2.rtc_id(), uri2)
//                     .offset(600000)
//                     .segments(recording2.segments().unwrap().to_owned())
//                     .modified_segments(recording2.segments().unwrap().to_owned())
//                     .pin_segments(vec![(Bound::Included(600000), Bound::Excluded(900000))].into()),
//             ],
//             host_stream_id: recording1.rtc_id(),
//         };

//         state
//             .tq_client_mock()
//             .expect_create_task()
//             .withf(move |class: &Class, task: &TqTask, _p: &Priority| {
//                 assert_eq!(class.id(), minigroup_id);
//                 assert_eq!(task, &expected_task);
//                 true
//             })
//             .returning(|_, _, _| Ok(()));

//         // Handle event room adjustment.
//         let state = Arc::new(state);

//         MinigroupPostprocessingStrategy::new(state.clone(), minigroup)
//             .handle_adjust(RoomAdjustResult::Success {
//                 original_room_id: original_event_room_id,
//                 modified_room_id: modified_event_room_id,
//                 cut_original_segments: cut_original_segments.clone(),
//                 modified_segments,
//             })
//             .await
//             .expect("Failed to handle event room adjustment");

//         // Assert DB changes.
//         let mut conn = state.get_conn().await.expect("Failed to get conn");

//         let updated_minigroup = MinigroupReadQuery::by_id(minigroup_id)
//             .execute(&mut conn)
//             .await
//             .expect("Failed to fetch minigroup")
//             .expect("Minigroup not found");

//         assert_eq!(
//             updated_minigroup.original_event_room_id(),
//             Some(original_event_room_id),
//         );

//         assert_eq!(
//             updated_minigroup.modified_event_room_id(),
//             Some(modified_event_room_id),
//         );

//         let recordings = RecordingListQuery::new(minigroup_id)
//             .execute(&mut conn)
//             .await
//             .expect("Failed to fetch recordings");

//         for recording in [&recording1, &recording2] {
//             let updated_recording = recordings
//                 .iter()
//                 .find(|r| r.id() == recording.id())
//                 .expect("Missing recording");

//             assert!(updated_recording.adjusted_at().is_some());

//             assert_eq!(
//                 updated_recording.modified_segments(),
//                 if recording.id() == recording1.id() {
//                     Some(&cut_original_segments)
//                 } else {
//                     recording.segments()
//                 }
//             );
//         }
//     }

//     #[tokio::test]
//     async fn handle_adjust_with_pin_and_unpin() {
//         let now = Utc::now();
//         let agent1 = TestAgent::new("web", "user1", USR_AUDIENCE);
//         let agent2 = TestAgent::new("web", "user2", USR_AUDIENCE);
//         let mut state = TestState::new(TestAuthz::new()).await;
//         let event_room_id = Uuid::new_v4();
//         let original_event_room_id = Uuid::new_v4();
//         let modified_event_room_id = Uuid::new_v4();
//         let conference_room_id = Uuid::new_v4();

//         let original_host_segments: Segments = vec![
//             (Bound::Included(0), Bound::Excluded(1_500_000)),
//             (Bound::Included(1_800_000), Bound::Excluded(3_000_000)),
//         ]
//         .into();

//         // assume there was single cut-stop at the beginning
//         let modified_segments: Segments =
//             vec![(Bound::Included(0), Bound::Excluded(2_697_000))].into();

//         let cut_original_segments: Segments = vec![
//             (Bound::Included(3_000), Bound::Excluded(1_500_000)),
//             (Bound::Included(1_800_000), Bound::Excluded(3_000_000)),
//         ]
//         .into();

//         // Insert a minigroup with recordings.
//         let (minigroup, recording1, recording2) = {
//             let mut conn = state.get_conn().await.expect("Failed to get conn");

//             let time = (
//                 Bound::Included(now - Duration::hours(1)),
//                 Bound::Excluded(now - Duration::minutes(10)),
//             );

//             let minigroup_scope = format!("minigroup-{}", random_string());

//             let minigroup = factory::Minigroup::new(
//                 minigroup_scope,
//                 USR_AUDIENCE.to_string(),
//                 time.into(),
//                 conference_room_id,
//                 event_room_id,
//             )
//             .insert(&mut conn)
//             .await;

//             let recording1 = factory::Recording::new(
//                 minigroup.id(),
//                 Uuid::new_v4(),
//                 agent1.agent_id().to_owned(),
//             )
//             .segments(original_host_segments.clone())
//             .started_at(now - Duration::hours(1))
//             .stream_uri("s3://minigroup.origin.dev.example.com/rtc1.webm".to_string())
//             .insert(&mut conn)
//             .await;

//             let recording2 = factory::Recording::new(
//                 minigroup.id(),
//                 Uuid::new_v4(),
//                 agent2.agent_id().to_owned(),
//             )
//             .segments(vec![(Bound::Included(0), Bound::Excluded(2700000))].into())
//             .stream_uri("s3://minigroup.origin.dev.example.com/rtc2.webm".to_string())
//             .started_at(now - Duration::minutes(50))
//             .insert(&mut conn)
//             .await;

//             (minigroup, recording1, recording2)
//         };

//         let minigroup_id = minigroup.id();

//         state
//             .conference_client_mock()
//             .expect_read_config_snapshots()
//             .with(mockall::predicate::eq(conference_room_id))
//             .returning(move |_room_id| Ok(vec![]));

//         // Set up event client mock.
//         state
//             .event_client_mock()
//             .expect_read_room()
//             .with(mockall::predicate::eq(modified_event_room_id))
//             .returning(move |room_id| {
//                 Ok(EventRoomResponse {
//                     id: room_id,
//                     time: (
//                         Bound::Included(now - Duration::hours(1)),
//                         Bound::Excluded(now - Duration::minutes(10)),
//                     ),
//                     tags: None,
//                 })
//             });

//         state
//             .event_client_mock()
//             .expect_dump_room()
//             .with(mockall::predicate::eq(modified_event_room_id))
//             .returning(move |_room_id| Ok(()));

//         state
//             .event_client_mock()
//             .expect_list_events()
//             .withf(move |room_id: &Uuid, _kind: &str| {
//                 assert_eq!(*room_id, modified_event_room_id);
//                 true
//             })
//             .returning(move |_, kind| match kind {
//                 PIN_EVENT_TYPE => Ok(vec![
//                     EventBuilder::new()
//                         .room_id(modified_event_room_id)
//                         .set(PIN_EVENT_TYPE.to_string())
//                         .data(EventData::Pin(PinEventData::new(
//                             agent1.agent_id().to_owned(),
//                         )))
//                         .occurred_at(0)
//                         .build(),
//                     EventBuilder::new()
//                         .room_id(modified_event_room_id)
//                         .set(PIN_EVENT_TYPE.to_string())
//                         .data(EventData::Pin(PinEventData::null()))
//                         .occurred_at(1000000000000)
//                         .build(),
//                 ]),
//                 HOST_EVENT_TYPE => Ok(vec![EventBuilder::new()
//                     .room_id(modified_event_room_id)
//                     .set(HOST_EVENT_TYPE.to_string())
//                     .data(EventData::Host(HostEventData::new(
//                         agent1.agent_id().to_owned(),
//                     )))
//                     .occurred_at(0)
//                     .build()]),
//                 other => panic!("Event client mock got unknown kind: {}", other),
//             });

//         // Set up tq client mock.
//         let uri1 = recording1.stream_uri().unwrap().to_string();
//         let uri2 = recording2.stream_uri().unwrap().to_string();

//         let expected_task = TqTask::TranscodeMinigroupToHls {
//             streams: vec![
//                 TranscodeMinigroupToHlsStream::new(recording1.rtc_id(), uri1)
//                     .offset(0)
//                     .segments(original_host_segments.clone())
//                     .modified_segments(cut_original_segments.clone())
//                     .pin_segments(vec![(Bound::Included(0), Bound::Excluded(1_000_000))].into()),
//                 TranscodeMinigroupToHlsStream::new(recording2.rtc_id(), uri2)
//                     .offset(600_000)
//                     .segments(recording2.segments().unwrap().to_owned())
//                     .modified_segments(recording2.segments().unwrap().to_owned())
//                     .pin_segments(vec![].into()),
//             ],
//             host_stream_id: recording1.rtc_id(),
//         };

//         state
//             .tq_client_mock()
//             .expect_create_task()
//             .withf(move |class: &Class, task: &TqTask, _p: &Priority| {
//                 assert_eq!(class.id(), minigroup_id);
//                 assert_eq!(task, &expected_task);
//                 true
//             })
//             .returning(|_, _, _| Ok(()));

//         // Handle event room adjustment.
//         let state = Arc::new(state);

//         MinigroupPostprocessingStrategy::new(state.clone(), minigroup)
//             .handle_adjust(RoomAdjustResult::Success {
//                 original_room_id: original_event_room_id,
//                 modified_room_id: modified_event_room_id,
//                 cut_original_segments: cut_original_segments.clone(),
//                 modified_segments,
//             })
//             .await
//             .expect("Failed to handle event room adjustment");

//         // Assert DB changes.
//         let mut conn = state.get_conn().await.expect("Failed to get conn");

//         let updated_minigroup = MinigroupReadQuery::by_id(minigroup_id)
//             .execute(&mut conn)
//             .await
//             .expect("Failed to fetch minigroup")
//             .expect("Minigroup not found");

//         assert_eq!(
//             updated_minigroup.original_event_room_id(),
//             Some(original_event_room_id),
//         );

//         assert_eq!(
//             updated_minigroup.modified_event_room_id(),
//             Some(modified_event_room_id),
//         );

//         let recordings = RecordingListQuery::new(minigroup_id)
//             .execute(&mut conn)
//             .await
//             .expect("Failed to fetch recordings");

//         for recording in [&recording1, &recording2] {
//             let updated_recording = recordings
//                 .iter()
//                 .find(|r| r.id() == recording.id())
//                 .expect("Missing recording");

//             assert!(updated_recording.adjusted_at().is_some());

//             assert_eq!(
//                 updated_recording.modified_segments(),
//                 if recording.id() == recording1.id() {
//                     Some(&cut_original_segments)
//                 } else {
//                     recording.segments()
//                 }
//             );
//         }
//     }
// }

mod handle_transcoding_completion {
    use std::ops::Bound;
    use std::sync::Arc;

    use chrono::{Duration, Utc};
    use serde_json::json;
    use uuid::Uuid;

    use crate::app::{AppContext, API_VERSION};
    use crate::db::recording::{RecordingListQuery, Segments};
    use crate::test_helpers::{prelude::*, shared_helpers::random_string};

    use super::super::super::PostprocessingStrategy;
    use super::super::*;

    #[tokio::test]
    async fn handle_transcoding_completion() {
        let now = Utc::now();
        let agent1 = TestAgent::new("web", "user1", USR_AUDIENCE);
        let agent2 = TestAgent::new("web", "user2", USR_AUDIENCE);
        let state = TestState::new(TestAuthz::new()).await;

        // Insert a minigroup with recordings.
        let (minigroup, recording1, recording2) = {
            let mut conn = state.get_conn().await.expect("Failed to get conn");

            let time = (
                Bound::Included(now - Duration::hours(1)),
                Bound::Excluded(now - Duration::minutes(10)),
            );

            let minigroup_scope = format!("minigroup-{}", random_string());

            let minigroup = factory::Minigroup::new(
                minigroup_scope,
                USR_AUDIENCE.to_string(),
                time.into(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
            .original_event_room_id(Uuid::new_v4())
            .modified_event_room_id(Uuid::new_v4())
            .tags(json!({ "foo": "bar" }))
            .insert(&mut conn)
            .await;

            let segments1: Segments = vec![
                (Bound::Included(0), Bound::Excluded(1500000)),
                (Bound::Included(1800000), Bound::Excluded(3000000)),
            ]
            .into();

            let recording1 = factory::Recording::new(
                minigroup.id(),
                Uuid::new_v4(),
                agent1.agent_id().to_owned(),
            )
            .stream_uri("s3://minigroup.origin.dev.example.com/rtc1.webm".to_string())
            .segments(segments1)
            .started_at(now - Duration::hours(1))
            .insert(&mut conn)
            .await;

            let recording2 = factory::Recording::new(
                minigroup.id(),
                Uuid::new_v4(),
                agent2.agent_id().to_owned(),
            )
            .stream_uri("s3://minigroup.origin.dev.example.com/rtc2.webm".to_string())
            .segments(vec![(Bound::Included(0), Bound::Excluded(2700000))].into())
            .started_at(now - Duration::minutes(50))
            .insert(&mut conn)
            .await;

            (minigroup, recording1, recording2)
        };

        // Handle event room adjustment.
        let state = Arc::new(state);

        MinigroupPostprocessingStrategy::new(state.clone(), minigroup.clone())
            .handle_transcoding_completion(TranscodeSuccess::TranscodeMinigroupToHls(
                TranscodeMinigroupToHlsSuccess {
                    recording_duration: "3000.0".to_string(),
                },
            ))
            .await
            .expect("Failed to handle tq transcoding completion");

        // Assert DB changes.
        let mut conn = state.get_conn().await.expect("Failed to get conn");

        let updated_recordings = RecordingListQuery::new(minigroup.id())
            .execute(&mut conn)
            .await
            .expect("Failed to list recordings");

        for recording in &[recording1, recording2] {
            let updated_recording = updated_recordings
                .iter()
                .find(|r| r.id() == recording.id())
                .expect("Recording not found");

            assert!(updated_recording.transcoded_at().is_some());
        }

        // Assert outgoing audience-level event.
        let messages = state.test_publisher().flush();
        let message = messages.first().expect("No event published");

        assert_eq!(
            message.topic(),
            format!(
                "apps/{}/api/{}/audiences/{}/events",
                state.config().id,
                API_VERSION,
                USR_AUDIENCE
            ),
        );

        match message.properties() {
            OutgoingEnvelopeProperties::Event(evp) => {
                assert_eq!(evp.label(), "minigroup.ready");
            }
            props => panic!("Unexpected message properties: {:?}", props),
        }

        assert_eq!(
            message.payload::<MinigroupReady>(),
            MinigroupReady {
                id: minigroup.id(),
                scope: minigroup.scope().to_owned(),
                tags: minigroup.tags().map(ToOwned::to_owned),
                status: "success".to_string(),
                stream_duration: 3000,
            }
        );
    }
}
