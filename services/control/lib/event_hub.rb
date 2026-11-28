# frozen_string_literal: true

require "json"

# Server-Sent Events fan-out for the management plane.
#
# Deviation from docs/MASTER-ARCHITECTURE.md §19 (WebSocket): Studio only
# needs server→client events, SSE needs no extra protocol stack, and it
# works through Puma's ordinary streaming bodies. Browsers consume it with
# fetch() + ReadableStream (EventSource cannot send the bearer token, and
# tokens never go into URLs).
#
# Bounds: at most MAX_CLIENTS subscribers, each with a QUEUE_SIZE queue. A
# full queue marks a slow consumer, which is disconnected; it reconnects and
# reconciles from the `snapshot` event sent first on every stream. One slow
# browser can therefore never block publishers or other clients.
#
# Fairness: one credential (session or token) holds at most
# MAX_PER_PRINCIPAL streams and one account MAX_PER_USER; a new stream
# beyond that replaces the same principal's oldest one (a reconnecting
# client also frees the slot its dead connection still held). Non-admins
# cannot take the last RESERVED_FOR_ADMIN slots.
class EventHub
  MAX_CLIENTS = 8
  MAX_PER_PRINCIPAL = 3
  MAX_PER_USER = 4
  RESERVED_FOR_ADMIN = 2
  QUEUE_SIZE = 64
  HEARTBEAT_SECONDS = 15

  class Subscription
    attr_reader :queue, :user_id, :session, :principal, :admin

    # user_id/session: the account session that opened the stream, so it can
    # be closed when that session ends or the account changes. principal:
    # the credential (for per-principal limits). check: called every
    # heartbeat; the stream ends once it returns false (revoked token,
    # expired session).
    def initialize(user_id: nil, session: nil, principal: nil, admin: true, check: nil)
      @queue = SizedQueue.new(QUEUE_SIZE)
      @user_id = user_id
      @session = session
      @principal = principal
      @admin = admin
      @check = check
    end
    def closed? = @queue.closed?
    def close = @queue.close

    def valid?
      @check.nil? || @check.call == true
    rescue StandardError
      false
    end
  end

  def initialize
    @mutex = Mutex.new
    @subs = []
  end

  def subscriber_count = @mutex.synchronize { @subs.size }
  def subscriptions = @mutex.synchronize { @subs.dup }

  # Returns a Subscription or nil when the client limit is reached.
  def subscribe(user_id: nil, session: nil, principal: nil, admin: true, check: nil)
    replaced = []
    sub = @mutex.synchronize do
      # Oldest first, so the oldest stream of the same principal is replaced.
      own = principal && @subs.select { |s| s.principal == principal }
      replaced << own.first if own && own.size >= MAX_PER_PRINCIPAL
      mine = user_id && (@subs - replaced).select { |s| s.user_id == user_id }
      replaced << mine.first if mine && mine.size >= MAX_PER_USER
      @subs -= replaced
      limit = admin ? MAX_CLIENTS : MAX_CLIENTS - RESERVED_FOR_ADMIN
      if @subs.size >= limit
        nil
      else
        Subscription.new(user_id: user_id, session: session, principal: principal, admin: admin, check: check)
          .tap { |s| @subs << s }
      end
    end
    replaced.each(&:close)
    sub
  end

  # Ends the streams matching the block (sign-out, disabled account, ...).
  def close_where(&match)
    subs = @mutex.synchronize { @subs.select(&match).tap { |m| @subs -= m } }
    subs.each(&:close)
    subs.size
  end

  def unsubscribe(sub)
    @mutex.synchronize { @subs.delete(sub) }
    sub.close
  end

  # Never blocks; returns the number of subscribers dropped as slow.
  def publish(event, data)
    frame = self.class.frame(event, data)
    dropped = []
    @mutex.synchronize do
      @subs.each do |sub|
        sub.queue.push(frame, true)
      rescue ThreadError, ClosedQueueError
        dropped << sub
      end
      @subs -= dropped
    end
    dropped.each(&:close)
    dropped.size
  end

  def close_all
    subs = @mutex.synchronize { @subs.dup.tap { @subs.clear } }
    subs.each(&:close)
  end

  def self.frame(event, data) = "event: #{event}\ndata: #{JSON.generate(data)}\n\n"

  # Rack streaming body for one subscriber. Puma writes each yielded chunk
  # immediately (chunked encoding); a failed write (client gone) ends `each`
  # and Puma calls #close, which releases the slot. The subscription's
  # credential is re-checked every `recheck` seconds, also while events
  # keep flowing.
  class Stream
    def initialize(hub, sub, snapshot, heartbeat: HEARTBEAT_SECONDS, recheck: HEARTBEAT_SECONDS)
      @hub = hub
      @sub = sub
      @snapshot = snapshot
      @heartbeat = heartbeat
      @recheck = recheck
    end

    def each
      yield "retry: 2000\n\n"
      yield EventHub.frame("snapshot", @snapshot)
      next_check = clock + @recheck
      loop do
        frame = @sub.queue.pop(timeout: [@heartbeat, @recheck].min)
        break if frame.nil? && @sub.closed?

        if clock >= next_check
          break unless @sub.valid?

          next_check = clock + @recheck
        end
        yield frame || ": keepalive\n\n"
      end
    end

    def close = @hub.unsubscribe(@sub)

    private

    def clock = Process.clock_gettime(Process::CLOCK_MONOTONIC)
  end

  # Polls engine.status and publishes transitions. Runs in its own thread so
  # requests never wait on it.
  class EnginePoller
    attr_reader :last

    # `on_connected` is called with the status whenever an engine appears or
    # restarts, so the control plane can re-apply state the engine does not
    # persist (audio configuration).
    def initialize(hub, engine_provider, interval: 1.0, on_connected: nil)
      @hub = hub
      @engine_provider = engine_provider
      @on_connected = on_connected
      @interval = interval
      @last = nil
      @polled = false
      @mutex = Mutex.new
    end

    def start
      @mutex.synchronize do
        @thread ||= Thread.new do
          Thread.current.name = "stephania-events"
          loop do
            begin
              tick
            rescue StandardError => e
              warn "event poller: #{e.class}: #{e.message}"
            end
            sleep @interval
          end
        end
      end
      self
    end

    def stop
      @mutex.synchronize do
        @thread&.kill
        @thread = nil
      end
    end

    # One poll; public for tests.
    def tick
      status = begin
        @engine_provider.call.call("engine.status")
      rescue EngineClient::Unavailable, EngineClient::RemoteError
        nil
      end
      previous = @last
      first = !@polled
      @last = status
      @polled = true

      if status.nil?
        @hub.publish("engine.unavailable", {}) if first || previous
        return
      end
      # The instance seen last survives outages, so an engine that comes
      # back as a new process is reported as restarted, not merely connected.
      seen = @instance_id
      @instance_id = status["instance_id"]
      begin
        @on_connected&.call(status) if previous.nil? || seen != @instance_id
      rescue StandardError => e
        warn "event poller: on_connected: #{e.class}: #{e.message}"
      end
      if seen && seen != @instance_id
        return @hub.publish("engine.restarted", status) if previous.nil?

        @hub.publish("engine.restarted", status)
      elsif previous.nil?
        return @hub.publish("engine.connected", status)
      end
      keys = %w[state streaming_requested native_worker_running last_error]
      @hub.publish("stream.state_changed", status.slice(*keys)) if previous.slice(*keys) != status.slice(*keys)
      @hub.publish("program.changed", status["program"]) if status["program"] && previous["program"] != status["program"]
    end
  end

  # Program audio meters while streaming: `audio.levels` at ~4 Hz, only
  # when someone is listening (docs/AUDIO.md).
  class LevelPoller
    def initialize(hub, engine_provider, interval: 0.25)
      @hub = hub
      @engine_provider = engine_provider
      @interval = interval
      @mutex = Mutex.new
    end

    def start
      @mutex.synchronize do
        @thread ||= Thread.new do
          Thread.current.name = "stephania-levels"
          loop do
            tick if @hub.subscriber_count.positive?
            sleep @interval
          end
        end
      end
      self
    end

    def stop
      @mutex.synchronize do
        @thread&.kill
        @thread = nil
      end
    end

    def tick
      status = @engine_provider.call.call("audio.status")
      return unless status["running"]

      inputs = status["inputs"].map { |i| i.slice("id", "state", "levels", "muted", "gain_db") }
      @hub.publish("audio.levels", {inputs: inputs, master: status["master"]})
    rescue EngineClient::Unavailable, EngineClient::RemoteError
      nil
    end
  end
end
