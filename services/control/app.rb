# frozen_string_literal: true

require "sinatra/base"
require "json"
require "sequel"
require "openssl"
require "securerandom"
require "tmpdir"
require_relative "lib/engine_client"
require_relative "lib/scene_validator"
require_relative "lib/event_hub"
require_relative "lib/lan_access"
require_relative "lib/audio_settings"
require_relative "lib/accounts"
require_relative "lib/asset_store"
require_relative "lib/template_validator"

# T3Box management plane. Contract: contracts/openapi.yaml.
#
# Never carries media. Talks to the engine only through EngineClient
# (Unix socket IPC). Plain HTTP is loopback-only (T3BOX_CONTROL_TOKEN).
# The LAN is served only over TLS with the device-CA certificate
# (crate::pki) and accepts only owner-issued tokens enrolled over
# authenticated BLE (crate::management_tokens); see LanAccess.
class Control < Sinatra::Base
  MAX_BODY_BYTES = 65_536
  MAX_REVISIONS_KEPT = 50
  MIGRATIONS = File.expand_path("db/migrations", __dir__)
  ID = /\A[a-zA-Z0-9_-]{1,64}\z/
  ALLOWED_HOSTS = %w[localhost 127.0.0.1].freeze
  CAMERA_ROUTES = [%w[POST /api/v1/cameras/session/offer], %w[GET /api/v1/cameras/session], %w[DELETE /api/v1/cameras/session]].freeze
  MAX_CAMERA_TTL = 86_400
  MAX_CAMERA_CREDENTIALS = 8
  LOGIN_ROUTE = %w[POST /api/v1/auth/session].freeze
  # Operator role (docs/AUTH.md): run the show, change nothing structural.
  # Reads are allowed except accounts and credentials.
  OPERATOR_WRITES = [%w[POST /api/v1/stream/start], %w[POST /api/v1/stream/stop], %w[POST /api/v1/program/transition],
    %w[PUT /api/v1/audio], %w[DELETE /api/v1/auth/session], %w[PUT /api/v1/auth/password]].freeze
  ADMIN_ONLY_READS = %r{\A/api/v1/(users|credentials)(/|\z)}

  # Engine IPC error code -> HTTP status. Unknown codes are a contract
  # violation by the engine and surface as 502.
  ENGINE_STATUS = {
    "engine_start_failed" => 503,
    "busy" => 503,
    "invalid_scene" => 422,
    "not_implemented" => 501,
    "program_inactive" => 409,
    "program_timeout" => 503
  }.freeze

  set :bind, "127.0.0.1"
  set :public_folder, ENV.fetch("T3BOX_STUDIO_DIST", File.expand_path("../../apps/studio/dist", __dir__))
  set :show_exceptions, false
  set :raise_errors, false
  set :dump_errors, false
  set :logging, false
  set :host_authorization, {permitted_hosts: []} if Sinatra::VERSION.to_i >= 4

  # Dependencies are plain class attributes, not Sinatra settings: `set`
  # treats any Proc value as a lazily evaluated getter and calls it with no
  # arguments, which is what turned the injected engine adapter into an
  # HTTP 500 in the first foundation CI run.
  class << self
    attr_accessor :db, :token, :engine, :events, :poller, :levels, :lan, :phone, :assets_dir
    attr_writer :throttle

    def throttle = (@throttle ||= Accounts::Throttle.new)
    # Camera credential id that opened the current phone session (nil when
    # the owner opened it); revoking that credential ends the session.
    attr_accessor :phone_session_owner

    def configure_storage(path)
      database = Sequel.sqlite(path, max_connections: 1, timeout: 1000)
      database.run("PRAGMA foreign_keys = ON")
      database.run("PRAGMA journal_mode = WAL") unless path == ":memory:"
      Sequel.extension :migration
      Sequel::Migrator.run(database, MIGRATIONS)
      self.db = database
      @engine_instance = nil
    end

    # Restart reconciliation: remember the last engine instance seen and
    # report whether it changed. Thread-safe under multi-threaded Puma.
    def observe_engine_instance(instance_id)
      @engine_mutex ||= Mutex.new
      @engine_mutex.synchronize do
        previous = @engine_instance
        @engine_instance = instance_id
        !previous.nil? && previous != instance_id
      end
    end

    def configure_from_env(env = ENV)
      self.token = env["T3BOX_CONTROL_TOKEN"]
      self.lan = LanAccess.new(pki_dir: env["T3BOX_PKI_DIR"], tokens_file: env["T3BOX_CONTROL_TOKENS_FILE"])
      self.engine = EngineClient.new(socket_path: env.fetch("T3BOX_ENGINE_SOCKET", EngineClient::DEFAULT_SOCKET))
      # t3box-phone speaks the same framing (docs/PHONE.md); answers wait for
      # ICE gathering, hence the longer timeout.
      self.phone = EngineClient.new(socket_path: env.fetch("T3BOX_PHONE_SOCKET", "/run/t3box-phone/phone.sock"), timeout: 8.0)
      self.assets_dir = env.fetch("T3BOX_ASSETS_DIR") { File.join(Dir.tmpdir, "t3box-assets-#{Process.pid}") }
      configure_storage(env.fetch("T3BOX_CONTROL_DB", ":memory:"))
      configure_events(interval: Float(env.fetch("T3BOX_CONTROL_POLL_SECONDS", "1.0")))
    end

    # The poller reads `Control.engine` on every tick, so tests that swap
    # the engine adapter are observed without restarting it.
    def configure_events(interval: 1.0)
      poller&.stop
      events&.close_all
      self.events = EventHub.new
      self.poller = EventHub::EnginePoller.new(events, -> { Control.engine }, interval: interval,
        on_connected: ->(_status) { Control.apply_audio })
      levels&.stop
      self.levels = EventHub::LevelPoller.new(events, -> { Control.engine })
    end

    # Pushes the stored audio configuration to the engine (which does not
    # persist it). Returns the engine's audio.status, or nil if nothing is
    # stored. Raises EngineClient errors to the caller.
    def apply_audio
      params = AudioSettings.params(AudioSettings.load(db))
      params && engine.call("audio.configure", params)
    end
  end

  configure { configure_from_env }

  helpers do
    def request_id
      @request_id ||= SecureRandom.hex(8)
    end

    def json_reply(value, code = 200)
      status code
      JSON.generate(value)
    end

    # Consistent error envelope: {"error":{"code","message","request_id"}}.
    def fail_with(code, error, message)
      halt code, {"Content-Type" => "application/json"},
        JSON.generate(error: {code: error, message: message, request_id: request_id})
    end

    def read_json
      fail_with 413, "body_too_large", "request body exceeds #{MAX_BODY_BYTES} bytes" if request.content_length.to_i > MAX_BODY_BYTES
      raw = request.body.read(MAX_BODY_BYTES + 1).to_s
      fail_with 413, "body_too_large", "request body exceeds #{MAX_BODY_BYTES} bytes" if raw.bytesize > MAX_BODY_BYTES
      JSON.parse(raw)
    rescue JSON::ParserError
      fail_with 400, "invalid_json", "request body is not valid JSON"
    end

    def engine_call(method, params = nil)
      settings.engine.call(method, params, request_id: request_id)
    rescue EngineClient::RemoteError => e
      fail_with ENGINE_STATUS.fetch(e.code, 502), e.code, e.message
    rescue EngineClient::Unavailable => e
      fail_with 503, "engine_unavailable", "media engine is not reachable (#{e.message})"
    end

    CAMERA_TOKEN = /\At3c_[0-9a-f]{64}\z/

    # Valid, unexpired camera credential row for a presented token, or nil.
    def camera_credential(supplied)
      return nil unless supplied.match?(CAMERA_TOKEN)

      row = settings.db[:camera_credentials].where(token_sha256: OpenSSL::Digest::SHA256.hexdigest(supplied)).first
      row if row && row[:expires_at_unix] > Time.now.to_i
    end

    def credential_json(row)
      {id: row[:id], label: row[:label], created_at_unix: row[:created_at_unix], expires_at_unix: row[:expires_at_unix]}
    end

    PHONE_STATUS = {"busy" => 409, "invalid_offer" => 422, "invalid_params" => 422}.freeze

    def phone_call(method, params = nil)
      settings.phone.call(method, params, request_id: request_id)
    rescue EngineClient::RemoteError => e
      fail_with PHONE_STATUS.fetch(e.code, 502), e.code, e.message
    rescue EngineClient::Unavailable
      fail_with 503, "phone_unavailable", "phone media service is not reachable"
    end

    def scene_row(id)
      fail_with 400, "invalid_id", "scene id must match #{ID.source}" unless id.match?(ID)
      settings.db[:scenes].where(id: id).first
    end

    # Lowercase slug of `base` that is not taken yet ("name", "name-2", ...).
    def unique_scene_id(base)
      slug = base.to_s.downcase.gsub(/[^a-z0-9_-]+/, "-").gsub(/\A-+|-+\z/, "")[0, 48]
      slug = "scene" if slug.empty?
      taken = settings.db[:scenes].where(Sequel.like(:id, "#{slug}%")).select_map(:id)
      return slug unless taken.include?(slug)

      (2..).each { |n| return "#{slug}-#{n}" unless taken.include?("#{slug}-#{n}") }
    end

    # Scenes may only reference uploaded assets.
    def check_assets!(document)
      ids = AssetStore.referenced(document)
      return if ids.empty?

      known = settings.db[:assets].where(id: ids).select_map(:id)
      missing = ids - known
      fail_with 422, "unknown_asset", "no such asset: #{missing.join(", ")}" unless missing.empty?
    end

    def create_scene(base, document)
      if (violation = SceneValidator.validate(document))
        fail_with 422, "invalid_scene", violation.join(": ")
      end
      check_assets!(document)
      db = settings.db
      raw = JSON.generate(document)
      id = nil
      db.transaction(mode: :immediate) do
        id = unique_scene_id(base)
        db[:scenes].insert(id: id, revision: 1, document: raw)
        db[:scene_revisions].insert(scene_id: id, revision: 1, document: raw, created_at_unix: Time.now.to_i)
      end
      settings.events.publish("scene.updated", {id: id, revision: 1})
      {id: id, revision: 1, document: document}
    end

    def template_json(row)
      {id: row[:id], name: row[:name], width: row[:width], height: row[:height], svg: row[:svg], vars: JSON.parse(row[:vars])}
    end

    def asset_json(row)
      {id: row[:id], name: row[:name], width: row[:width], height: row[:height], bytes: row[:bytes], created_at_unix: row[:created_at_unix]}
    end

    def scene_json(row)
      {id: row[:id], revision: row[:revision], document: JSON.parse(row[:document])}
    end
  end

  before do
    headers "Cache-Control" => "no-store", "X-Content-Type-Options" => "nosniff",
      "X-Request-Id" => request_id,
      "Content-Security-Policy" => "default-src 'self'; img-src 'self' blob: data:; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'"
    @lan_request = !ALLOWED_HOSTS.include?(request.host)
    if @lan_request
      # Non-loopback Host values are accepted only over TLS and only for the
      # names/addresses in the device certificate (DNS-rebinding defence).
      fail_with 403, "forbidden_host", "LAN access requires TLS to a certified device name" unless request.ssl? && settings.lan&.lan_host?(request.host)
      headers "Strict-Transport-Security" => "max-age=31536000"
    end
    origin = request.env["HTTP_ORIGIN"]
    fail_with 403, "forbidden_origin", "cross-origin requests are not allowed" if origin && origin != "#{request.scheme}://#{request.host_with_port}"
  end

  before "/api/*" do
    content_type :json
    # Login is the one unauthenticated API call (host/origin checks above
    # still apply, so on the LAN it is TLS-only).
    next if request.request_method == LOGIN_ROUTE[0] && request.path_info == LOGIN_ROUTE[1]

    token = settings.token
    env_token = token.is_a?(String) && token.bytesize >= 32
    unless env_token || settings.lan&.configured?
      fail_with 503, "not_configured", "T3BOX_CONTROL_TOKEN must be at least 32 bytes"
    end
    header = request.env.fetch("HTTP_AUTHORIZATION", "")
    supplied = header.delete_prefix("Bearer ")
    bearer = header.start_with?("Bearer ")
    # The bootstrap token is loopback-only; the LAN accepts only tokens the
    # owner enrolled over BLE (revoked by factory reset).
    local = !@lan_request && bearer && env_token && supplied.bytesize == token.bytesize &&
      OpenSSL.fixed_length_secure_compare(supplied, token)
    owner = local || (bearer && settings.lan&.issued_token?(supplied))
    @camera = !owner && bearer && camera_credential(supplied)
    if owner
      @principal = {kind: "owner", role: "admin"}
    elsif !bearer && (found = Accounts.lookup_session(settings.db, request.cookies[Accounts::COOKIE]))
      session, user = found
      # Cookie sessions need the CSRF token on every state-changing call,
      # in addition to the strict Origin check and SameSite=Strict.
      unless %w[GET HEAD].include?(request.request_method)
        csrf = request.env.fetch("HTTP_X_CSRF_TOKEN", "")
        unless csrf.bytesize == session[:csrf_token].bytesize && OpenSSL.fixed_length_secure_compare(csrf, session[:csrf_token])
          fail_with 403, "csrf_failed", "missing or invalid X-CSRF-Token"
        end
      end
      @session = session
      @principal = {kind: "user", role: user[:role], user_id: user[:id], username: user[:username]}
    end
    fail_with 401, "unauthorized", "missing or invalid credentials" unless @principal || @camera
    # Camera role: media session only (§26).
    if @camera && !CAMERA_ROUTES.any? { |verb, path| request.request_method == verb && request.path_info == path }
      fail_with 403, "forbidden_role", "camera credentials only allow the camera session"
    end
    if @principal && @principal[:role] == "operator"
      allowed = if %w[GET HEAD].include?(request.request_method)
        !request.path_info.match?(ADMIN_ONLY_READS)
      else
        OPERATOR_WRITES.any? { |verb, path| request.request_method == verb && request.path_info == path }
      end
      fail_with 403, "forbidden_role", "operators cannot do this" unless allowed
    end
  end

  get "/api/v1/capabilities" do
    engine = begin
      settings.engine.call("engine.capabilities", nil, request_id: request_id)
    rescue EngineClient::Unavailable, EngineClient::RemoteError
      {}
    end
    json_reply({scene_drafts: true, program_apply: engine["program_apply"] == true, phone_camera: true,
                audio_mixer: true, accounts: true, engine_reachable: !engine.empty?})
  end

  # --- Accounts and sessions (docs/AUTH.md) -------------------------------

  post "/api/v1/auth/session" do
    input = read_json
    unless input.is_a?(Hash) && input.keys.sort == %w[password username] && input["username"].is_a?(String) && input["password"].is_a?(String)
      fail_with 422, "invalid_request", "body must be {username, password}"
    end
    name = input["username"].downcase
    keys = ["user:#{name}", "ip:#{request.ip}"]
    if (wait = settings.throttle.wait(keys)).positive?
      headers "Retry-After" => wait.to_s
      fail_with 429, "login_throttled", "too many failed logins; retry in #{wait} s"
    end
    user = name.match?(Accounts::USERNAME) ? settings.db[:users].where(username: name).first : nil
    # Same scrypt work whether or not the user exists.
    ok = Accounts.verify_password(input["password"], user ? user[:password_hash] : Accounts::DUMMY_HASH) && user && !user[:disabled]
    unless ok
      settings.throttle.failure(keys)
      fail_with 401, "invalid_login", "unknown user or wrong password"
    end
    settings.throttle.success(keys)
    value, session = Accounts.create_session(settings.db, user[:id])
    response.set_cookie(Accounts::COOKIE, value: value, path: "/api/", httponly: true, secure: request.ssl?,
      same_site: :strict, expires: Time.at(session[:expires_at_unix]))
    json_reply({user: Accounts.user_json(user), csrf_token: session[:csrf_token], expires_at_unix: session[:expires_at_unix]}, 201)
  end

  delete "/api/v1/auth/session" do
    if @session
      settings.db[:sessions].where(token_sha256: @session[:token_sha256]).delete
      settings.events.close_where { |sub| sub.session == @session[:token_sha256] }
    end
    response.delete_cookie(Accounts::COOKIE, path: "/api/", httponly: true, secure: request.ssl?, same_site: :strict)
    status 204
    ""
  end

  get "/api/v1/auth/me" do
    fail_with 403, "forbidden_role", "camera credentials have no account" unless @principal
    me = {kind: @principal[:kind], role: @principal[:role]}
    if @session
      me[:username] = @principal[:username]
      me[:user_id] = @principal[:user_id]
      me[:csrf_token] = @session[:csrf_token]
      me[:expires_at_unix] = @session[:expires_at_unix]
    end
    json_reply(me)
  end

  put "/api/v1/auth/password" do
    fail_with 422, "invalid_request", "only signed-in users have a password" unless @session
    input = read_json
    unless input.is_a?(Hash) && input.keys.sort == %w[current_password new_password]
      fail_with 422, "invalid_request", "body must be {current_password, new_password}"
    end
    user = settings.db[:users].where(id: @principal[:user_id]).first
    fail_with 403, "invalid_login", "current password is wrong" unless Accounts.verify_password(input["current_password"], user[:password_hash])
    if (problem = Accounts.password_problem(input["new_password"]))
      fail_with 422, "weak_password", problem
    end
    db = settings.db
    db.transaction do
      db[:users].where(id: user[:id]).update(password_hash: Accounts.hash_password(input["new_password"]))
      # Every other session of this user ends.
      db[:sessions].where(user_id: user[:id]).exclude(token_sha256: @session[:token_sha256]).delete
    end
    settings.events.close_where { |sub| sub.user_id == user[:id] && sub.session != @session[:token_sha256] }
    status 204
    ""
  end

  get "/api/v1/users" do
    json_reply(settings.db[:users].order(:id).all.map { |r| Accounts.user_json(r) })
  end

  post "/api/v1/users" do
    input = read_json
    unless input.is_a?(Hash) && input.keys.sort == %w[password role username] && input["username"].is_a?(String) &&
        input["username"].match?(Accounts::USERNAME) && Accounts::ROLES.include?(input["role"])
      fail_with 422, "invalid_request", "body must be {username (#{Accounts::USERNAME.source}), password, role (admin|operator)}"
    end
    if (problem = Accounts.password_problem(input["password"]))
      fail_with 422, "weak_password", problem
    end
    db = settings.db
    fail_with 409, "user_exists", "username is taken" if db[:users].where(username: input["username"]).count.positive?
    id = db[:users].insert(username: input["username"], password_hash: Accounts.hash_password(input["password"]),
      role: input["role"], disabled: false, created_at_unix: Time.now.to_i)
    json_reply(Accounts.user_json(db[:users].where(id: id).first), 201)
  end

  patch "/api/v1/users/:id" do
    id = Integer(params[:id], exception: false)
    db = settings.db
    user = id && db[:users].where(id: id).first
    fail_with 404, "not_found", "no such user" unless user
    input = read_json
    unless input.is_a?(Hash) && !input.empty? && (input.keys - %w[role disabled password]).empty? &&
        (input["role"].nil? || Accounts::ROLES.include?(input["role"])) &&
        (input["disabled"].nil? || [true, false].include?(input["disabled"]))
      fail_with 422, "invalid_request", "body may contain role, disabled, password"
    end
    if input.key?("password") && (problem = Accounts.password_problem(input["password"]))
      fail_with 422, "weak_password", problem
    end
    demoting = user[:role] == "admin" && !user[:disabled] && ((input["role"] && input["role"] != "admin") || input["disabled"] == true)
    fail_with 409, "last_admin", "at least one enabled admin must remain" if demoting && Accounts.admins_remaining(db, user[:id]).zero?
    changes = {}
    changes[:role] = input["role"] if input["role"]
    changes[:disabled] = input["disabled"] unless input["disabled"].nil?
    changes[:password_hash] = Accounts.hash_password(input["password"]) if input["password"]
    db.transaction do
      db[:users].where(id: user[:id]).update(changes)
      # A changed password, role or disabled account ends existing sessions.
      db[:sessions].where(user_id: user[:id]).delete
    end
    settings.events.close_where { |sub| sub.user_id == user[:id] }
    json_reply(Accounts.user_json(db[:users].where(id: user[:id]).first))
  end

  delete "/api/v1/users/:id" do
    id = Integer(params[:id], exception: false)
    db = settings.db
    user = id && db[:users].where(id: id).first
    fail_with 404, "not_found", "no such user" unless user
    if user[:role] == "admin" && !user[:disabled] && Accounts.admins_remaining(db, user[:id]).zero?
      fail_with 409, "last_admin", "at least one enabled admin must remain"
    end
    db[:users].where(id: user[:id]).delete
    settings.events.close_where { |sub| sub.user_id == user[:id] }
    status 204
    ""
  end

  get "/api/v1/status" do
    result = engine_call("engine.status")
    restarted = Control.observe_engine_instance(result["instance_id"])
    json_reply(result.merge("engine_restarted" => restarted))
  end

  # Server-Sent Events (text/event-stream). First event is `snapshot`
  # ({engine, scenes}); then engine.connected / engine.unavailable /
  # engine.restarted / stream.state_changed / program.changed /
  # scene.updated / scene.deleted, plus comment keepalives. Bounded to
  # EventHub::MAX_CLIENTS concurrent streams (429 beyond).
  get "/api/v1/events" do
    hub = settings.events
    settings.poller.start
    settings.levels.start
    snapshot = {engine: settings.poller.last, scenes: settings.db[:scenes].order(:id).select(:id, :revision).all}
    sub = hub.subscribe(user_id: @principal&.dig(:user_id), session: @session&.dig(:token_sha256))
    fail_with 429, "too_many_streams", "at most #{EventHub::MAX_CLIENTS} event streams" unless sub
    headers "Content-Type" => "text/event-stream", "X-Accel-Buffering" => "no"
    halt 200, EventHub::Stream.new(hub, sub, snapshot)
  end

  get "/api/v1/engine/capabilities" do
    json_reply(engine_call("engine.capabilities"))
  end

  post "/api/v1/stream/start" do
    json_reply(engine_call("stream.start"), 202)
  end

  post "/api/v1/stream/stop" do
    json_reply(engine_call("stream.stop"))
  end

  get "/api/v1/scenes" do
    json_reply(settings.db[:scenes].order(:id).all.map { |row| scene_json(row) })
  end

  get "/api/v1/scenes/:id" do
    row = scene_row(params[:id])
    fail_with 404, "not_found", "scene does not exist" unless row
    json_reply(scene_json(row))
  end

  get "/api/v1/scenes/:id/revisions" do
    row = scene_row(params[:id])
    fail_with 404, "not_found", "scene does not exist" unless row
    revisions = settings.db[:scene_revisions].where(scene_id: row[:id]).reverse(:revision).all
    json_reply(revisions.map { |r| {revision: r[:revision], created_at_unix: r[:created_at_unix], document: JSON.parse(r[:document])} })
  end

  delete "/api/v1/scenes/:id" do
    row = scene_row(params[:id])
    fail_with 404, "not_found", "scene does not exist" unless row
    expected = Integer(params["revision"], exception: false)
    fail_with 422, "invalid_request", "revision query parameter required" unless expected
    deleted = settings.db[:scenes].where(id: row[:id], revision: expected).delete
    fail_with 409, "revision_conflict", "scene revision is #{row[:revision]}" if deleted.zero?
    settings.events.publish("scene.deleted", {id: row[:id]})
    status 204
    ""
  end

  # New scene from a name (and optionally a document). The id is derived
  # from the name and made unique; revision 1 is recorded in the history.
  post "/api/v1/scenes" do
    input = read_json
    unless input.is_a?(Hash) && (input.keys - %w[name document]).empty? && input["name"].is_a?(String)
      fail_with 422, "invalid_request", "body must be {name, document?}"
    end
    document = input["document"] || {"version" => 2, "name" => input["name"],
      "layers" => [{"id" => "hdmi", "type" => "video", "source" => "hdmi", "x" => 0, "y" => 0, "width" => 1920, "height" => 1080, "visible" => true}]}
    document = document.merge("name" => input["name"]) if document.is_a?(Hash)
    json_reply(create_scene(input["name"], document), 201)
  end

  post "/api/v1/scenes/:id/duplicate" do
    row = scene_row(params[:id])
    fail_with 404, "not_found", "scene does not exist" unless row
    document = JSON.parse(row[:document])
    document["name"] = "#{document["name"]} (copy)"[0, 100]
    json_reply(create_scene(row[:id], document), 201)
  end

  # Portable export: {format, version, id, document}. Importing never
  # overwrites; it creates a new scene.
  get "/api/v1/scenes/:id/export" do
    row = scene_row(params[:id])
    fail_with 404, "not_found", "scene does not exist" unless row
    attachment "#{row[:id]}.t3box-scene.json"
    json_reply({format: "t3box-scene", version: 1, id: row[:id], document: JSON.parse(row[:document])})
  end

  post "/api/v1/scenes/import" do
    input = read_json
    unless input.is_a?(Hash) && input["format"] == "t3box-scene" && input["version"] == 1 &&
        (input.keys - %w[format version id document]).empty? && input["document"].is_a?(Hash)
      fail_with 422, "invalid_request", "body must be a t3box-scene export (format, version 1, document)"
    end
    base = input["id"].is_a?(String) ? input["id"] : input["document"]["name"].to_s
    json_reply(create_scene(base, input["document"]), 201)
  end

  # Draft save with optimistic concurrency. `revision` is the revision the
  # client last saw (0 to create). Saving a draft never changes Program.
  put "/api/v1/scenes/:id" do
    id = params[:id]
    scene_row(id)
    input = read_json
    unless input.is_a?(Hash) && input.keys.sort == %w[document revision] &&
        input["revision"].is_a?(Integer) && input["revision"] >= 0
      fail_with 422, "invalid_request", "body must be {revision, document}"
    end
    if (violation = SceneValidator.validate(input["document"]))
      fail_with 422, "invalid_scene", violation.join(": ")
    end
    check_assets!(input["document"])
    revision = input["revision"]
    document = JSON.generate(input["document"])
    db = settings.db
    saved = db.transaction(mode: :immediate) do
      row = db[:scenes].where(id: id).first
      if (row && row[:revision] != revision) || (!row && revision != 0)
        false
      else
        if row
          db[:scenes].where(id: id).update(revision: revision + 1, document: document)
        else
          db[:scenes].insert(id: id, revision: 1, document: document)
        end
        db[:scene_revisions].insert(scene_id: id, revision: revision + 1, document: document, created_at_unix: Time.now.to_i)
        cutoff = revision + 1 - MAX_REVISIONS_KEPT
        db[:scene_revisions].where(scene_id: id).where(Sequel[:revision] <= cutoff).delete
        true
      end
    end
    fail_with 409, "revision_conflict", "scene was modified; reload and retry" unless saved
    settings.events.publish("scene.updated", {id: id, revision: revision + 1})
    json_reply({id: id, revision: revision + 1, document: input["document"]})
  end

  # Request Program transition to a stored draft revision. The engine
  # validates independently and answers only after the media thread
  # acknowledged frame-boundary application (first_frame_index). Without the
  # compositor (501), without a running stream (409 program_inactive) or on
  # timeout (503) Program is unchanged.
  post "/api/v1/program/transition" do
    input = read_json
    unless input.is_a?(Hash) && (input.keys - %w[transition]).sort == %w[revision scene_id] &&
        input["scene_id"].is_a?(String) && input["revision"].is_a?(Integer)
      fail_with 422, "invalid_request", "body must be {scene_id, revision, transition?}"
    end
    transition = input["transition"]
    unless transition.nil? || transition == {"type" => "cut"} ||
        (transition.is_a?(Hash) && transition.keys.sort == %w[duration_ms type] && transition["type"] == "fade" &&
         transition["duration_ms"].is_a?(Integer) && transition["duration_ms"].between?(1, 5000))
      fail_with 422, "invalid_request", "transition must be {type: cut} or {type: fade, duration_ms: 1..5000}"
    end
    row = scene_row(input["scene_id"])
    fail_with 404, "not_found", "scene does not exist" unless row
    fail_with 409, "revision_conflict", "scene revision is #{row[:revision]}" unless row[:revision] == input["revision"]
    apply = {scene: JSON.parse(row[:document]), scene_id: row[:id], revision: row[:revision]}
    apply[:transition] = transition if transition
    result = engine_call("scene.apply", apply)
    # Only reachable once the engine reports an acknowledged application.
    settings.events.publish("program.changed", {scene_id: row[:id], revision: row[:revision]})
    json_reply({scene_id: row[:id], revision: row[:revision], engine: result})
  end

  # --- Image assets (docs/SCENE.md) ---------------------------------------
  # Raw PNG upload (Content-Type: image/png), content-addressed: uploading
  # the same file twice returns the existing asset.
  post "/api/v1/assets" do
    fail_with 415, "unsupported_media_type", "upload must be image/png" unless request.media_type == "image/png"
    if request.content_length.to_i > AssetStore::MAX_BYTES
      fail_with 413, "body_too_large", "assets are limited to #{AssetStore::MAX_BYTES} bytes"
    end
    data = request.body.read(AssetStore::MAX_BYTES + 1).to_s.b
    fail_with 413, "body_too_large", "assets are limited to #{AssetStore::MAX_BYTES} bytes" if data.bytesize > AssetStore::MAX_BYTES
    dims = AssetStore.inspect_png(data)
    fail_with 422, "invalid_asset", dims if dims.is_a?(String)
    name = params["name"].to_s[0, 100]
    name = "image.png" unless name.match?(/\A[[:print:]]{1,100}\z/)
    db = settings.db
    id = AssetStore.id_for(data)
    if (row = db[:assets].where(id: id).first)
      halt 200, JSON.generate(asset_json(row))
    end
    fail_with 409, "too_many_assets", "at most #{AssetStore::MAX_ASSETS} assets" if db[:assets].count >= AssetStore::MAX_ASSETS
    AssetStore.write(settings.assets_dir, id, data)
    row = {id: id, sha256: OpenSSL::Digest::SHA256.hexdigest(data), name: name, width: dims[0], height: dims[1],
           bytes: data.bytesize, created_at_unix: Time.now.to_i}
    db[:assets].insert(row)
    json_reply(asset_json(row), 201)
  end

  get "/api/v1/assets" do
    json_reply(settings.db[:assets].order(:created_at_unix, :id).all.map { |r| asset_json(r) })
  end

  get "/api/v1/assets/:id" do
    id = params[:id]
    fail_with 400, "invalid_id", "asset id must be 16 hex digits" unless id.match?(AssetStore::ID)
    fail_with 404, "not_found", "no such asset" unless settings.db[:assets].where(id: id).count.positive?
    path = AssetStore.path(settings.assets_dir, id)
    fail_with 404, "not_found", "asset file is missing" unless File.file?(path)
    headers "Cache-Control" => "private, max-age=31536000, immutable"
    send_file path, type: "image/png"
  end

  delete "/api/v1/assets/:id" do
    id = params[:id]
    fail_with 400, "invalid_id", "asset id must be 16 hex digits" unless id.match?(AssetStore::ID)
    db = settings.db
    fail_with 404, "not_found", "no such asset" unless db[:assets].where(id: id).count.positive?
    users = db[:scenes].all.select { |r| AssetStore.referenced(JSON.parse(r[:document])).include?(id) }.map { |r| r[:id] }
    fail_with 409, "asset_in_use", "used by scene #{users.join(", ")}" unless users.empty?
    db[:assets].where(id: id).delete
    File.unlink(AssetStore.path(settings.assets_dir, id)) if File.exist?(AssetStore.path(settings.assets_dir, id))
    status 204
    ""
  end

  # --- Developer overlay templates (docs/SCENE.md) -----------------------
  get "/api/v1/templates" do
    json_reply(settings.db[:templates].order(:id).all.map { |r| template_json(r) })
  end

  put "/api/v1/templates/:id" do
    id = params[:id]
    input = read_json
    if (problem = TemplateValidator.problem(id, input))
      fail_with 422, "invalid_template", problem
    end
    db = settings.db
    fail_with 409, "too_many_templates", "at most 64 templates" if db[:templates].count >= 64 && db[:templates].where(id: id).count.zero?
    row = {id: id, name: input["name"], width: input["width"], height: input["height"], svg: input["svg"],
           vars: JSON.generate(input["vars"]), updated_at_unix: Time.now.to_i}
    db.transaction do
      db[:templates].where(id: id).delete
      db[:templates].insert(row)
    end
    json_reply(template_json(row))
  end

  delete "/api/v1/templates/:id" do
    id = params[:id]
    fail_with 400, "invalid_id", "template id must match #{TemplateValidator::ID.source}" unless id.match?(TemplateValidator::ID)
    fail_with 404, "not_found", "no such template" if settings.db[:templates].where(id: id).delete.zero?
    status 204
    ""
  end

  # Phone camera session (docs/PHONE.md). The control plane relays SDP
  # only; media flows phone → t3box-phone (WebRTC) → engine, never here.
  post "/api/v1/cameras/session/offer" do
    input = read_json
    unless input.is_a?(Hash) && (input.keys - %w[sdp tracks replace]).empty? && input["sdp"].is_a?(String) &&
        input["tracks"].is_a?(Hash) && (input["replace"].nil? || [true, false].include?(input["replace"]))
      fail_with 422, "invalid_request", "body must be {sdp, tracks, replace?}"
    end
    params = {sdp: input["sdp"], tracks: input["tracks"]}
    params[:replace] = input["replace"] unless input["replace"].nil?
    # A camera may not take over a session opened by someone else.
    if @camera && params[:replace] && Control.phone_session_owner != @camera[:id]
      fail_with 403, "forbidden_role", "a camera credential cannot replace another session"
    end
    result = phone_call("session.offer", params)
    Control.phone_session_owner = @camera && @camera[:id]
    json_reply(result)
  end

  get "/api/v1/cameras/session" do
    json_reply(phone_call("session.status"))
  end

  delete "/api/v1/cameras/session" do
    if @camera && Control.phone_session_owner != @camera[:id]
      fail_with 403, "forbidden_role", "a camera credential can only end its own session"
    end
    phone_call("session.close")
    Control.phone_session_owner = nil
    status 204
    ""
  end

  # Camera credentials (owner only): short-lived tokens that can do nothing
  # but run the phone camera session. Shown once; stored as SHA-256.
  post "/api/v1/credentials/camera" do
    input = read_json
    ttl = input.is_a?(Hash) ? input.fetch("ttl_s", 12 * 3600) : nil
    label = input.is_a?(Hash) ? input.fetch("label", "camera") : nil
    unless input.is_a?(Hash) && (input.keys - %w[ttl_s label]).empty? && ttl.is_a?(Integer) && ttl.between?(60, MAX_CAMERA_TTL) &&
        label.is_a?(String) && label.match?(/\A[[:print:]]{1,64}\z/)
      fail_with 422, "invalid_request", "body must be {ttl_s: 60..#{MAX_CAMERA_TTL}, label: 1..64 printable}"
    end
    now = Time.now.to_i
    db = settings.db
    db[:camera_credentials].where(Sequel[:expires_at_unix] <= now).delete
    fail_with 409, "too_many_credentials", "at most #{MAX_CAMERA_CREDENTIALS} camera credentials" if db[:camera_credentials].count >= MAX_CAMERA_CREDENTIALS
    token = "t3c_#{SecureRandom.hex(32)}"
    row = {id: SecureRandom.hex(4), token_sha256: OpenSSL::Digest::SHA256.hexdigest(token), label: label,
           created_at_unix: now, expires_at_unix: now + ttl}
    db[:camera_credentials].insert(row)
    json_reply(credential_json(row).merge(token: token), 201)
  end

  get "/api/v1/credentials/camera" do
    rows = settings.db[:camera_credentials].where(Sequel[:expires_at_unix] > Time.now.to_i).order(:created_at_unix).all
    json_reply(rows.map { |r| credential_json(r) })
  end

  delete "/api/v1/credentials/camera/:id" do
    id = params[:id]
    fail_with 400, "invalid_id", "credential id must be 8 hex digits" unless id.match?(/\A[0-9a-f]{8}\z/)
    fail_with 404, "not_found", "no such credential" if settings.db[:camera_credentials].where(id: id).delete.zero?
    if Control.phone_session_owner == id
      begin
        settings.phone.call("session.close", nil, request_id: request_id)
      rescue EngineClient::Unavailable, EngineClient::RemoteError
        nil # the session ends with the service anyway
      end
      Control.phone_session_owner = nil
    end
    status 204
    ""
  end

  # Program audio inputs (docs/AUDIO.md): stored configuration plus the
  # engine's live health and levels.
  get "/api/v1/audio" do
    stored = AudioSettings.load(settings.db)
    live = begin
      settings.engine.call("audio.status", nil, request_id: request_id)
    rescue EngineClient::Unavailable, EngineClient::RemoteError
      nil
    end
    json_reply({stored: stored.values, engine: live})
  end

  # Partial update ({inputs: [{id, ...changed fields}]}). Saved first, then
  # applied live; if the engine is down the configuration is applied when
  # it reconnects (EnginePoller on_connected).
  put "/api/v1/audio" do
    db = settings.db
    input = read_json
    base = AudioSettings.load(db)
    # Inputs never configured here start from the engine's current settings
    # (its built-in defaults), so a partial update keeps e.g. the HDMI device.
    if base.size < AudioSettings::IDS.size
      begin
        live = settings.engine.call("audio.status", nil, request_id: request_id)
        Array(live["inputs"]).each do |i|
          base[i["id"]] ||= i.slice(*AudioSettings::FIELDS) if AudioSettings::IDS.include?(i["id"])
        end
      rescue EngineClient::Unavailable, EngineClient::RemoteError
        nil
      end
    end
    merged, error = AudioSettings.validate(input, base)
    fail_with 422, "invalid_audio", error if error
    applied = begin
      settings.engine.call("audio.configure", AudioSettings.params(merged), request_id: request_id)
    rescue EngineClient::RemoteError => e
      fail_with 422, "invalid_audio", e.message
    rescue EngineClient::Unavailable
      nil
    end
    AudioSettings.save(db, merged)
    settings.events.publish("audio.configured", {inputs: merged.values})
    json_reply({stored: merged.values, engine: applied, applied: !applied.nil?})
  end

  # Device CA certificate (public, no credentials). Browsers on a PC need it
  # installed; the phone app shows the SHA-256 it received sealed over BLE so
  # the user can compare before trusting it (docs/AUTH.md).
  get "/ca.pem" do
    path = settings.lan&.ca_path
    fail_with 404, "not_found", "device PKI is not configured" unless path && File.file?(path)
    headers "Content-Disposition" => 'attachment; filename="t3box-ca.pem"'
    send_file path, type: "application/x-pem-file"
  end

  get "/" do
    index = File.join(settings.public_folder, "index.html")
    fail_with 404, "not_found", "studio is not built" unless File.file?(index)
    send_file index
  end

  not_found do
    content_type :json
    JSON.generate(error: {code: "not_found", message: "no such route", request_id: request_id})
  end

  error do
    content_type :json
    status 500
    JSON.generate(error: {code: "internal_error", message: "unexpected server error", request_id: request_id})
  end
end
