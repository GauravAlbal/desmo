import importlib.util
import os

import pytest


@pytest.fixture()
def app():
    spec = importlib.util.spec_from_file_location("app_under_test", os.path.join(os.environ["APP_DIR"], "app.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


@pytest.fixture()
def r(app, tmp_path):
    return app.Refunds(str(tmp_path / "state"))


def test_happy_path_and_versions(r):
    s = r.submit("r1", "alice", 500)
    assert (s["state"], s["version"], s["amount_cents"]) == ("draft", 1, 500)
    s = r.approve("r1", "bob", 500)
    assert (s["state"], s["version"], s["approver"]) == ("approved", 2, "bob")
    s = r.authorize_payout("r1", 500)
    assert (s["state"], s["version"]) == ("payout_pending", 3)
    s = r.record_payout("r1", "d1")
    assert (s["state"], s["version"], s["paid_digest"]) == ("paid", 4, "d1")
    assert r.get("r1") == s


def test_self_approval_and_unknown_and_resubmit_refused(app, r):
    r.submit("r1", "alice", 500)
    with pytest.raises(app.Refused):
        r.approve("r1", "alice", 500)
    with pytest.raises(app.Refused):
        r.submit("r1", "bob", 1)
    with pytest.raises(app.Refused):
        r.get("nope")
    with pytest.raises(app.Refused):
        r.approve("nope", "bob", 1)
    assert r.get("r1")["version"] == 1


def test_approval_binds_exact_amount(app, r):
    r.submit("r1", "alice", 500)
    with pytest.raises(app.Refused):
        r.approve("r1", "bob", 499)
    assert r.get("r1")["state"] == "draft"


def test_edit_after_approval_discards_approval(app, r):
    r.submit("r1", "alice", 500)
    r.approve("r1", "bob", 500)
    s = r.edit("r1", 700)
    assert (s["state"], s["amount_cents"], s["approver"]) == ("draft", 700, None)
    with pytest.raises(app.Refused):
        r.authorize_payout("r1", 700)
    with pytest.raises(app.Refused):
        r.authorize_payout("r1", 500)
    r.approve("r1", "carol", 700)
    assert r.authorize_payout("r1", 700)["state"] == "payout_pending"


def test_authorize_requires_approved_amount_and_is_single(app, r):
    r.submit("r1", "alice", 500)
    with pytest.raises(app.Refused):
        r.authorize_payout("r1", 500)
    r.approve("r1", "bob", 500)
    with pytest.raises(app.Refused):
        r.authorize_payout("r1", 501)
    r.authorize_payout("r1", 500)
    with pytest.raises(app.Refused):
        r.authorize_payout("r1", 500)
    with pytest.raises(app.Refused):
        r.edit("r1", 1)
    assert r.get("r1")["version"] == 3


def test_record_payout_duplicate_noop_and_conflict(app, r):
    r.submit("r1", "alice", 500)
    with pytest.raises(app.Refused):
        r.record_payout("r1", "d1")
    r.approve("r1", "bob", 500)
    r.authorize_payout("r1", 500)
    a = r.record_payout("r1", "d1")
    assert r.record_payout("r1", "d1") == a
    with pytest.raises(app.Conflict):
        r.record_payout("r1", "d2")
    assert r.get("r1")["paid_digest"] == "d1" and r.get("r1")["version"] == 4
    with pytest.raises(app.Refused):
        r.edit("r1", 1)


def test_stale_expected_version_changes_nothing(app, r):
    r.submit("r1", "alice", 500)
    with pytest.raises(app.Stale):
        r.approve("r1", "bob", 500, expected_version=2)
    with pytest.raises(app.Stale):
        r.edit("r1", 9, expected_version=0)
    assert r.get("r1")["state"] == "draft" and r.get("r1")["version"] == 1
    assert r.approve("r1", "bob", 500, expected_version=1)["version"] == 2
    with pytest.raises(app.Stale):
        r.authorize_payout("r1", 500, expected_version=1)


def test_lost_reply_then_other_writer_replays_original(app, r):
    r.submit("r1", "alice", 500)
    first = r.approve("r1", "bob", 500, operation_id="op-a")
    r.edit("r1", 800)  # other writer moves it on
    again = r.approve("r1", "bob", 500, operation_id="op-a")
    assert again == first
    assert r.get("r1")["state"] == "draft" and r.get("r1")["version"] == 3
    with pytest.raises(app.Conflict):
        r.approve("r1", "carol", 500, operation_id="op-a")


def test_operation_id_replay_survives_reopen_and_does_not_double_authorize(app, tmp_path):
    p = str(tmp_path / "state")
    r = app.Refunds(p)
    r.submit("r1", "alice", 500, operation_id="s")
    r.approve("r1", "bob", 500)
    a = r.authorize_payout("r1", 500, operation_id="auth")
    r2 = app.Refunds(p)
    assert r2.authorize_payout("r1", 500, operation_id="auth") == a
    assert r2.get("r1")["version"] == 3


def test_process_death_and_reopen_mid_flow(app, tmp_path):
    p = str(tmp_path / "state")
    r = app.Refunds(p)
    r.submit("r1", "alice", 500)
    r.approve("r1", "bob", 500)
    r.authorize_payout("r1", 500)
    del r
    r2 = app.Refunds(p)
    assert r2.get("r1")["state"] == "payout_pending"
    r2.record_payout("r1", "d1")
    r3 = app.Refunds(p)
    assert r3.get("r1")["state"] == "paid" and r3.get("r1")["paid_digest"] == "d1"
    with pytest.raises(app.Conflict):
        r3.record_payout("r1", "other")


def test_refused_actions_change_nothing_across_reopen(app, tmp_path):
    p = str(tmp_path / "state")
    r = app.Refunds(p)
    r.submit("r1", "alice", 500)
    before = r.get("r1")
    for bad in (lambda: r.approve("r1", "alice", 500), lambda: r.approve("r1", "bob", 1), lambda: r.record_payout("r1", "x")):
        with pytest.raises(app.Refused):
            bad()
    assert app.Refunds(p).get("r1") == before
