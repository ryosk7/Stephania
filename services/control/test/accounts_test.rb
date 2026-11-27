# frozen_string_literal: true

require_relative "test_helper"

# Accounts, cookie sessions, CSRF, roles and scene CRUD (docs/AUTH.md).
class AccountsTest < Minitest::Test
  include Rack::Test::Methods

  TOKEN = "o" * 32
  PASSWORD = "correct horse battery"

  def app = Control

  def setup
    Control.configure_storage(":memory:")
    Control.token = TOKEN
    Control.throttle = Accounts::Throttle.new
    @engine = FakeEngine.new do |req|
      case req["method"]
      when "engine.capabilities" then FakeEngine.ok(req, {"program_apply" => true})
      when "stream.start" then FakeEngine.ok(req, {"accepted" => true})
      else FakeEngine.ok(req, {})
      end
    end
    Control.engine = EngineClient.new(socket_path: @engine.path)
    header "Host", "localhost"
    header "Content-Type", "application/json"
  end

  def teardown = @engine.close
  def body = JSON.parse(last_response.body)

  def as_owner = header("Authorization", "Bearer #{TOKEN}")
  def anonymous = header("Authorization", nil)

  def create_user(name, role, password = PASSWORD)
    as_owner
    post "/api/v1/users", JSON.generate(username: name, password: password, role: role)
    assert_equal 201, last_response.status, last_response.body
    anonymous
    body
  end

  def login(name, password = PASSWORD)
    anonymous
    post "/api/v1/auth/session", JSON.generate(username: name, password: password)
    @csrf = last_response.status == 201 ? body["csrf_token"] : nil
    last_response
  end

  def with_csrf = header("X-CSRF-Token", @csrf)

  def test_password_hashing_round_trip_and_rejects_tampering
    encoded = Accounts.hash_password(PASSWORD)
    assert encoded.start_with?("scrypt$16384$8$1$")
    assert Accounts.verify_password(PASSWORD, encoded)
    refute Accounts.verify_password("wrong password!", encoded)
    refute Accounts.verify_password(PASSWORD, encoded.sub(/\$[^$]+\z/, "$AAAA"))
    refute Accounts.verify_password(PASSWORD, "garbage")
    refute_equal encoded, Accounts.hash_password(PASSWORD), "salted"
  end

  def test_login_sets_strict_http_only_cookie_and_me_reports_user
    create_user("alice", "admin")
    res = login("alice")
    assert_equal 201, res.status, res.body
    cookie = res.headers["Set-Cookie"]
    assert_match(/t3box_session=[A-Za-z0-9_-]{43}/, cookie)
    assert_match(/httponly/i, cookie)
    assert_match(/samesite=strict/i, cookie)
    assert_match(%r{path=/api/}i, cookie)
    assert_equal "alice", body.dig("user", "username")
    refute body["user"].key?("password_hash")
    get "/api/v1/auth/me"
    assert_equal 200, last_response.status
    assert_equal({"kind" => "user", "role" => "admin", "username" => "alice"}, body.slice("kind", "role", "username"))
    assert_equal @csrf, body["csrf_token"]
    # Only a hash of the cookie is stored.
    value = cookie[/t3box_session=([^;]+)/, 1]
    assert_equal [Accounts.digest(value)], Control.db[:sessions].select_map(:token_sha256)
  end

  def test_wrong_password_and_unknown_user_look_the_same_and_are_throttled
    create_user("bob", "operator")
    a = login("bob", "wrong password!!")
    b = login("nobody", "wrong password!!")
    assert_equal [401, 401], [a.status, b.status]
    assert_equal JSON.parse(a.body)["error"]["code"], JSON.parse(b.body)["error"]["code"]
    # Five failures from this address (two above, three here) lock it.
    3.times { login("bob", "wrong password!!") }
    res = login("bob")
    assert_equal 429, res.status, "locked even with the right password"
    assert res.headers["Retry-After"].to_i.positive?
  end

  def test_csrf_required_for_cookie_writes_but_not_reads
    create_user("carol", "admin")
    login("carol")
    get "/api/v1/scenes"
    assert_equal 200, last_response.status
    post "/api/v1/scenes", JSON.generate(name: "Talk")
    assert_equal 403, last_response.status
    assert_equal "csrf_failed", body.dig("error", "code")
    with_csrf
    post "/api/v1/scenes", JSON.generate(name: "Talk")
    assert_equal 201, last_response.status, last_response.body
  end

  def test_operator_runs_the_show_but_cannot_edit_or_manage
    create_user("olga", "operator")
    login("olga")
    with_csrf
    post "/api/v1/stream/start"
    assert_equal 202, last_response.status, last_response.body
    get "/api/v1/scenes"
    assert_equal 200, last_response.status
    put "/api/v1/scenes/lt", JSON.generate(revision: 0, document: {version: 1, name: "x", layers: []})
    assert_equal 403, last_response.status
    assert_equal "forbidden_role", body.dig("error", "code")
    get "/api/v1/users"
    assert_equal 403, last_response.status
    get "/api/v1/credentials/camera"
    assert_equal 403, last_response.status
  end

  def test_logout_ends_session
    create_user("dave", "admin")
    login("dave")
    with_csrf
    delete "/api/v1/auth/session"
    assert_equal 204, last_response.status
    assert_empty Control.db[:sessions].all
    get "/api/v1/auth/me"
    assert_equal 401, last_response.status
  end

  def test_idle_and_absolute_expiry
    create_user("erin", "admin")
    login("erin")
    Control.db[:sessions].update(last_seen_unix: Time.now.to_i - Accounts::IDLE_SECONDS - 1)
    get "/api/v1/auth/me"
    assert_equal 401, last_response.status
    login("erin")
    Control.db[:sessions].update(expires_at_unix: Time.now.to_i - 1)
    get "/api/v1/auth/me"
    assert_equal 401, last_response.status
  end

  def test_disabling_or_changing_a_user_ends_their_sessions_and_last_admin_is_kept
    admin = create_user("frank", "admin")
    other = create_user("gina", "operator")
    login("gina")
    stream = Control.events.subscribe(user_id: other["id"])
    unrelated = Control.events.subscribe
    as_owner
    patch "/api/v1/users/#{other["id"]}", JSON.generate(disabled: true)
    assert_equal 200, last_response.status, last_response.body
    assert_empty Control.db[:sessions].where(user_id: other["id"]).all
    assert stream.closed?, "the disabled user's event stream ends"
    refute unrelated.closed?
    Control.events.unsubscribe(unrelated)
    assert_equal 401, login("gina").status
    as_owner
    patch "/api/v1/users/#{admin["id"]}", JSON.generate(role: "operator")
    assert_equal 409, last_response.status
    assert_equal "last_admin", body.dig("error", "code")
    delete "/api/v1/users/#{admin["id"]}"
    assert_equal 409, last_response.status
    delete "/api/v1/users/#{other["id"]}"
    assert_equal 204, last_response.status
  end

  def test_user_validation
    as_owner
    post "/api/v1/users", JSON.generate(username: "Bad Name", password: PASSWORD, role: "admin")
    assert_equal 422, last_response.status
    post "/api/v1/users", JSON.generate(username: "ok", password: "short", role: "admin")
    assert_equal "weak_password", body.dig("error", "code")
    post "/api/v1/users", JSON.generate(username: "ok", password: PASSWORD, role: "root")
    assert_equal 422, last_response.status
    create_user("hana", "admin")
    as_owner
    post "/api/v1/users", JSON.generate(username: "hana", password: PASSWORD, role: "admin")
    assert_equal 409, last_response.status
  end

  def test_change_own_password_keeps_this_session_and_ends_others
    create_user("ivan", "operator")
    first = login("ivan").headers["Set-Cookie"][/t3box_session=([^;]+)/, 1]
    login("ivan")
    with_csrf
    put "/api/v1/auth/password", JSON.generate(current_password: "nope nope nope", new_password: "another long secret")
    assert_equal 403, last_response.status
    put "/api/v1/auth/password", JSON.generate(current_password: PASSWORD, new_password: "another long secret")
    assert_equal 204, last_response.status, last_response.body
    refute_includes Control.db[:sessions].select_map(:token_sha256), Accounts.digest(first)
    assert_equal 1, Control.db[:sessions].count
    assert_equal 201, login("ivan", "another long secret").status
  end

  def test_bearer_owner_still_works_and_me_reports_owner
    as_owner
    get "/api/v1/auth/me"
    assert_equal({"kind" => "owner", "role" => "admin"}, body)
    get "/api/v1/capabilities"
    assert_equal true, body["program_apply"]
  end

  def test_scene_create_duplicate_export_import
    as_owner
    post "/api/v1/scenes", JSON.generate(name: "Keynote Talk")
    assert_equal 201, last_response.status, last_response.body
    assert_equal({"id" => "keynote-talk", "revision" => 1}, body.slice("id", "revision"))
    post "/api/v1/scenes", JSON.generate(name: "Keynote Talk")
    assert_equal "keynote-talk-2", body["id"]
    post "/api/v1/scenes/keynote-talk/duplicate"
    assert_equal 201, last_response.status
    assert_equal "keynote-talk-3", body["id"]
    assert_equal "Keynote Talk (copy)", body.dig("document", "name")
    get "/api/v1/scenes/keynote-talk/export"
    assert_match(/attachment; filename="keynote-talk.t3box-scene.json"/, last_response.headers["Content-Disposition"])
    exported = body
    assert_equal "t3box-scene", exported["format"]
    post "/api/v1/scenes/import", JSON.generate(exported)
    assert_equal 201, last_response.status
    assert_equal "keynote-talk-4", body["id"], "import never overwrites"
    post "/api/v1/scenes/import", JSON.generate(exported.merge("document" => {"version" => 1}))
    assert_equal 422, last_response.status
    assert_equal "invalid_scene", body.dig("error", "code")
    get "/api/v1/scenes/keynote-talk-4/revisions"
    assert_equal [1], body.map { |r| r["revision"] }
  end
end
