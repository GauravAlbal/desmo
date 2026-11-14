import importlib.util
import os

import pytest


@pytest.fixture()
def app():
    spec = importlib.util.spec_from_file_location("app_under_test", os.path.join(os.environ["APP_DIR"], "app.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def test_two_distinct_approvers_required(app, tmp_path):
    r = app.Refunds(str(tmp_path / "s"))
    r.submit("r1", "alice", 500)
    s = r.approve("r1", "bob", 500)
    assert s["state"] == "draft"  # one approval is not enough
    assert s["version"] == 2
    with pytest.raises(app.Refused):
        r.approve("r1", "bob", 500)  # same approver twice
    with pytest.raises(app.Refused):
        r.authorize_payout("r1", 500)
    s = r.approve("r1", "carol", 500)
    assert s["state"] == "approved"
    assert r.authorize_payout("r1", 500)["state"] == "payout_pending"


def test_requester_still_cannot_approve_and_edit_clears_all_approvals(app, tmp_path):
    p = str(tmp_path / "s")
    r = app.Refunds(p)
    r.submit("r1", "alice", 500)
    with pytest.raises(app.Refused):
        r.approve("r1", "alice", 500)
    r.approve("r1", "bob", 500)
    r.approve("r1", "carol", 500)
    r.edit("r1", 600)
    r2 = app.Refunds(p)
    assert r2.get("r1")["state"] == "draft"
    r2.approve("r1", "carol", 600)  # carol's old approval is gone, so one approval is not enough
    assert r2.get("r1")["state"] == "draft"
    assert r2.approve("r1", "bob", 600)["state"] == "approved"


def test_amount_binding_still_applies_per_approval(app, tmp_path):
    r = app.Refunds(str(tmp_path / "s"))
    r.submit("r1", "alice", 500)
    with pytest.raises(app.Refused):
        r.approve("r1", "bob", 499)
    assert r.get("r1")["version"] == 1
