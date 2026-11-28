# frozen_string_literal: true

# Storage of the stream destination (lib/output_store.rb). Needs no gems.
require "minitest/autorun"
require "tmpdir"
require "fileutils"
require_relative "../lib/output_store"

class OutputStoreTest < Minitest::Test
  KEY = "abcd-efgh-ijkl-mnop-wxyz"
  URL = "rtmps://a.rtmps.youtube.com/live2"

  def setup
    @dir = Dir.mktmpdir("stephania-output")
    @path = File.join(@dir, "output.json")
    @store = OutputStore.new(@path)
  end

  def teardown = FileUtils.remove_entry(@dir)

  def test_file_is_private_and_atomic
    File.umask(0o022)
    @store.save(URL, KEY, now: 100)
    assert_equal 0o600, File.stat(@path).mode & 0o777
    assert_equal ["output.json"], Dir.children(@dir), "no temporary file left behind"
    assert_equal "#{URL}/#{KEY}", OutputStore.new(@path).output_url, "survives a restart"
    @store.save("rtmps://b.rtmps.youtube.com/live2", "zzzz-yyyy-xxxx", now: 200)
    assert_equal 0o600, File.stat(@path).mode & 0o777
    assert_equal "rtmps://b.rtmps.youtube.com/live2/zzzz-yyyy-xxxx", @store.output_url
  end

  def test_views_never_contain_the_key
    @store.save(URL, KEY, now: 100)
    admin = @store.public_json(admin: true)
    assert_equal({configured: true, url: URL, key_last4: "wxyz", updated_at_unix: 100}, admin)
    assert_equal({configured: true}, @store.public_json(admin: false))
    [admin, @store.public_json(admin: false)].each { |view| refute_includes JSON.generate(view), "mnop" }
  end

  def test_clear
    @store.save(URL, KEY)
    @store.clear
    refute File.exist?(@path)
    assert_nil @store.output_url
    assert_equal({configured: false, url: nil, key_last4: nil, updated_at_unix: nil}, @store.public_json(admin: true))
    @store.clear # idempotent
  end

  def test_rejects_bad_destinations_without_echoing_the_key
    [
      ["rtmp://a.rtmp.youtube.com/live2", KEY],
      ["https://a.rtmps.youtube.com/live2", KEY],
      ["rtmps://a.rtmps.youtube.com/live2/#{KEY}", KEY],
      ["rtmps://a.rtmps.youtube.com", KEY],
      ["rtmps://127.0.0.1/live2", KEY],
      ["rtmps://localhost/live2", KEY],
      ["rtmps://user@a.rtmps.youtube.com/live2", KEY],
      ["rtmps://a.rtmps.youtube.com:0/live2", KEY],
      ["rtmps://a.rtmps.youtube.com:70000/live2", KEY],
      ["rtmps://-bad.example.com/live2", KEY],
      [URL, "short"],
      [URL, "has/slash-in-key"],
      [URL, "query?key=secretvalue"],
      [URL, "space in the key"],
      [URL, "x" * 257],
      [nil, KEY]
    ].each do |url, key|
      error = assert_raises(OutputStore::Invalid, "#{url} accepted") { @store.save(url, key) }
      refute_includes error.message, key
    end
    refute File.exist?(@path)
    @store.save("rtmps://ingest.example.net:8443/app_1", KEY)
    assert_equal "rtmps://ingest.example.net:8443/app_1/#{KEY}", @store.output_url
  end

  def test_malformed_file_counts_as_not_configured
    File.write(@path, "{not json")
    assert_nil @store.output_url
    File.write(@path, JSON.generate(url: "rtmp://x.example.com/live2", stream_key: KEY))
    assert_equal({configured: false}, @store.public_json(admin: false))
  end

  def test_memory_store_and_path_from_env
    memory = OutputStore.new(nil)
    memory.save(URL, KEY)
    assert_equal "#{URL}/#{KEY}", memory.output_url
    memory.clear
    assert_nil memory.output_url
    assert_nil OutputStore.path_from_env({})
    assert_equal "/var/lib/x/output.json", OutputStore.path_from_env({"STEPHANIA_CONTROL_DB" => "/var/lib/x/control.db"})
    assert_equal "/o.json", OutputStore.path_from_env({"STEPHANIA_CONTROL_OUTPUT_FILE" => "/o.json", "STEPHANIA_CONTROL_DB" => "/a/b.db"})
  end
end
