// Conformance test for the turbo-parsepdf N-API addon. Requires the addon to be
// built (`npm run build:cargo` or `npm run build`).

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";

import { parse, parseToHtml, parseToJson, parseToMarkdown } from "../index.js";

const here = dirname(fileURLToPath(import.meta.url));
const pdf = readFileSync(join(here, "real.pdf"));

test("parse returns structured pages", () => {
  const doc = parse(pdf);
  assert.equal(doc.version, "1.5");
  assert.equal(doc.pages.length, 1);
  const page = doc.pages[0];
  assert.equal(page.needs_ocr, false);
  const text = page.lines.map((l) => l.text).join("\n");
  assert.match(text, /turbo-parsepdf/);
});

test("parseToHtml renders paragraphs", () => {
  const html = parseToHtml(pdf);
  assert.match(html, /<!DOCTYPE html>/);
  assert.match(html, /turbo-parsepdf/);
});

test("parseToMarkdown renders text", () => {
  assert.match(parseToMarkdown(pdf), /turbo-parsepdf/);
});

test("parseToJson is valid JSON matching parse()", () => {
  const json = JSON.parse(parseToJson(pdf));
  assert.deepEqual(json, parse(pdf));
});

test("a non-PDF buffer throws a typed error", () => {
  assert.throws(() => parse(Buffer.from("not a pdf")), /InvalidHeader/);
});
