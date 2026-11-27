# frozen_string_literal: true

# Developer template validation; mirrors apps/studio/src/templates.ts
# (checkDeveloperSvg). The SVG is only ever rendered by the Studio through
# an <img> element (no scripts, no network); these rules keep stored
# templates honest and the failure visible early.
module TemplateValidator
  ID = /\A[a-z0-9-]{1,32}\z/
  KEY = /\A[a-z][a-z0-9_.]{0,31}\z/
  MAX_SVG = 20_000
  RESERVED = %w[lower-third title].freeze

  module_function

  def svg_problem(svg)
    return "svg must be a string of at most #{MAX_SVG} characters" unless svg.is_a?(String) && svg.length <= MAX_SVG
    return "svg must start with <svg> and end with </svg>" unless svg.match?(/\A\s*<svg[\s>]/i) && svg.match?(%r{</svg>\s*\z}i)
    return "scripts and embedded content are not allowed" if svg.match?(/<script|<(iframe|object|embed)|\son[a-z]+\s*=|javascript:|<!ENTITY|<!DOCTYPE/i)
    return "external references are not allowed" if svg.match?(/(href|src)\s*=\s*["']\s*(?!#|data:image\/)/i)
    return "external url() is not allowed" if svg.match?(/url\(\s*["']?\s*(?!#|data:image\/)/i)

    nil
  end

  # nil when valid, otherwise a message.
  def problem(id, doc)
    return "template id must match #{ID.source}" unless id.match?(ID)
    return "#{id} is a built-in template" if RESERVED.include?(id)
    return "body must be {name, width, height, svg, vars}" unless doc.is_a?(Hash) && doc.keys.sort == %w[height name svg vars width]
    return "name must be 1..100 characters" unless doc["name"].is_a?(String) && doc["name"].length.between?(1, 100)
    return "width must be 1..1920" unless doc["width"].is_a?(Integer) && doc["width"].between?(1, 1920)
    return "height must be 1..1080" unless doc["height"].is_a?(Integer) && doc["height"].between?(1, 1080)

    vars = doc["vars"]
    unless vars.is_a?(Array) && vars.size <= 16 && vars.all? { |v| v.is_a?(Hash) && v.keys.sort == %w[initial key label] &&
        v["key"].is_a?(String) && v["key"].match?(KEY) && v["label"].is_a?(String) && v["label"].length.between?(1, 100) &&
        v["initial"].is_a?(String) && v["initial"].length <= 200 }
      return "vars must be at most 16 {key, label, initial}"
    end
    return "variable keys must be unique" unless vars.map { |v| v["key"] }.uniq.size == vars.size

    svg_problem(doc["svg"])
  end
end
