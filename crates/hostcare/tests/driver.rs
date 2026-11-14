//! The driver on a real tokio runtime: commands carried to a platform and
//! answered from other threads, the relay asked, deadlines kept by tokio's
//! timer, and a delete waiting out the pass it stood down.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use moshpit_hostcare::driver::{Driver, Platform};
use moshpit_hostcare::{
    Budgets, Change, Clock, Config, Conn, HostCare, HttpReply, Proof, PushSupport, Settings,
};
use moshpit_seal::{Date, Uuid};
use support::*;

struct Fake {
    host: Mutex<Host>,
    /// How long the host takes to answer.
    delay: Duration,
    changes: Mutex<Vec<Change>>,
    registered: Mutex<usize>,
}

#[async_trait]
impl Platform for Fake {
    async fn exec(&self, _connection: Uuid, command: String) -> Result<Vec<u8>, String> {
        tokio::time::sleep(self.delay).await;
        self.host.lock().unwrap().answer(&command)
    }

    async fn post(&self, _url: String, body: String, _timeout: u32) -> Result<HttpReply, String> {
        let request: serde_json::Value = serde_json::from_str(&body).unwrap();
        let token = moshpit_seal::token::mint(
            MASTER,
            request["apnsToken"].as_str().unwrap(),
            request["conn"].as_str().unwrap(),
            1_762_800_000,
        );
        Ok(HttpReply {
            status: 200,
            body: format!("{{\"sendToken\":\"{token}\",\"iat\":1762800000}}").into_bytes(),
        })
    }

    fn register_for_push(&self) {
        *self.registered.lock().unwrap() += 1;
    }

    fn remember_declined(&self, _connection: Uuid) {}

    fn remember_unpaired(&self, _connection: Uuid, _unpaired: bool) {}

    fn changed(&self, change: Change) {
        self.changes.lock().unwrap().push(change);
    }
}

fn driver(host: Host, delay: Duration, budgets: Budgets) -> (Driver, Arc<Fake>, SharedStore) {
    let fake = Arc::new(Fake {
        host: Mutex::new(host),
        delay,
        changes: Mutex::new(Vec::new()),
        registered: Mutex::new(0),
    });
    let store = SharedStore::default();
    let machine = HostCare::new(
        Config {
            push: PushSupport::Apns {
                environment: "sandbox".into(),
            },
            autocare_off: false,
            budgets,
        },
        Box::new(store.clone()),
        Clock {
            ms: 0,
            date: Date::now(),
        },
    );
    let driver = Driver::new(machine, fake.clone(), tokio::runtime::Handle::current());
    driver.with(|m, k| {
        m.set_settings(
            Settings {
                relay_url: RELAY.into(),
                ..Settings::default()
            },
            k,
        );
        m.restore_device_token(Some(DEVICE_TOKEN.into()), k);
    });
    (driver, fake, store)
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..500 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never: {what}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_care_pass_pairs_a_hooked_host_through_the_platform() {
    let (driver, fake, store) =
        driver(Host::hooked(), Duration::from_millis(2), Budgets::default());
    let c = Conn {
        id: Uuid::new_v4(),
        label: "m1-pro".into(),
    };
    driver.with(|m, k| m.set_saved(&[c.id], Some(&[c.id]), k));
    driver.with(|m, k| m.tend(c.clone(), k));
    until("the pass ends", || !driver.read(|m| m.is_tending(c.id))).await;
    let phone = store
        .read()
        .into_iter()
        .next()
        .expect("paired on the phone");
    assert_eq!(phone.apns_env.as_deref(), Some("sandbox"));
    assert_eq!(
        fake.host.lock().unwrap().pairing_conf(c.id),
        Some(&moshpit_install::push_conf(&phone))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_self_test_that_never_arrives_times_out_on_tokios_clock() {
    let budgets = Budgets {
        self_test_ms: 150,
        ..Budgets::default()
    };
    let (driver, fake, _) = driver(Host::healthy(), Duration::from_millis(1), budgets);
    let c = Conn {
        id: Uuid::new_v4(),
        label: "m1-pro".into(),
    };
    let page = driver.with(|m, k| m.open_setup(c.clone(), None, k));
    until("the host is read", || {
        driver.read(|m| m.setup_view(page).unwrap().facts.is_some())
    })
    .await;
    driver.with(|m, k| m.setup_prove_push(page, k));
    until("the test gives up", || {
        driver.read(|m| matches!(m.setup_view(page).unwrap().push_proof, Proof::Failed(_)))
    })
    .await;
    assert!(fake.changes.lock().unwrap().contains(&Change::Page(page)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_delete_waits_out_the_pass_it_stood_down() {
    let (driver, _fake, _) = driver(
        Host::hooked(),
        Duration::from_millis(200),
        Budgets::default(),
    );
    let c = Conn {
        id: Uuid::new_v4(),
        label: "doomed".into(),
    };
    driver.with(|m, k| m.set_saved(&[c.id], Some(&[c.id]), k));
    driver.with(|m, k| m.tend(c.clone(), k));
    assert!(driver.read(|m| m.is_tending(c.id)));
    driver.with(|m, k| m.deletion_began(c.id, k)).unwrap();
    tokio::time::timeout(Duration::from_secs(1), driver.care_wind_down(c.id))
        .await
        .expect("the wait is bounded and the pass is cancelled at once");
    assert!(!driver.read(|m| m.is_tending(c.id)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_command_the_host_never_answers_is_given_up_on_tokios_clock() {
    let budgets = Budgets {
        care_exec_ms: 150,
        ..Budgets::default()
    };
    // Answers in an hour: never, for this test.
    let (driver, _fake, _) = driver(Host::hooked(), Duration::from_secs(3600), budgets);
    let c = Conn {
        id: Uuid::new_v4(),
        label: "hung".into(),
    };
    driver.with(|m, k| m.set_saved(&[c.id], Some(&[c.id]), k));
    driver.with(|m, k| m.tend(c.clone(), k));
    assert!(driver.read(|m| m.is_tending(c.id)));
    until("the pass gives up", || !driver.read(|m| m.is_tending(c.id))).await;
}

/// B377: a delete takes the connection's pairing off its host before the
/// session goes — waiting for it, but never long: on a host slower than
/// the budget, the wait ends at the budget and the delete goes on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_delete_waits_for_its_pairing_to_leave_the_host_but_never_long() {
    let c = Conn {
        id: Uuid::new_v4(),
        label: "m1-pro".into(),
    };
    for (delay, gone) in [
        (Duration::from_millis(2), true),
        (Duration::from_secs(20), false),
    ] {
        let (driver, fake, store) =
            driver(Host::hooked(), Duration::from_millis(2), Budgets::default());
        driver.with(|m, k| m.set_saved(&[c.id], Some(&[c.id]), k));
        driver.with(|m, k| m.tend(c.clone(), k));
        until("the pass ends", || !driver.read(|m| m.is_tending(c.id))).await;
        assert!(fake.host.lock().unwrap().pairing_conf(c.id).is_some());
        // The host answers the delete's commands after `delay`.
        let slow = Arc::new(Fake {
            host: Mutex::new(std::mem::take(&mut *fake.host.lock().unwrap())),
            delay,
            changes: Mutex::new(Vec::new()),
            registered: Mutex::new(0),
        });
        drop(driver);
        let machine = HostCare::new(
            Config {
                push: PushSupport::Apns {
                    environment: "sandbox".into(),
                },
                autocare_off: false,
                budgets: Budgets::default(),
            },
            Box::new(store.clone()),
            Clock {
                ms: 0,
                date: Date::now(),
            },
        );
        let driver = Driver::new(machine, slow.clone(), tokio::runtime::Handle::current());
        driver.with(|m, k| m.set_saved(&[c.id], Some(&[c.id]), k));
        let started = std::time::Instant::now();
        driver.with(|m, k| m.deletion_began(c.id, k)).unwrap();
        driver.forget_wind_down(c.id).await;
        let waited = started.elapsed();
        assert!(waited < Duration::from_millis(3_500), "waited {waited:?}");
        assert_eq!(
            slow.host.lock().unwrap().pairing_conf(c.id).is_none(),
            gone,
            "{delay:?}"
        );
    }
}
