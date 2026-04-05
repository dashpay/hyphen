import assert from "node:assert/strict";
import test from "node:test";

import type { I18nMetaLanguage } from "@repo/hyphen-i18n";

import { applyValidatorTermOverridesForSelection } from "../../src/lib/config/validatorTerms.ts";

const language: I18nMetaLanguage = {
  dict: {
    masternode: "masternode",
    masternodes: "masternodes",
    masternodes_active_count:
      "{totalCount, plural, one {# masternode} other {# masternodes}} ({enabledCount} active)",
    masternodes_count: "{count, plural, one {# masternode} other {# masternodes}}",
    masternodes_voted: "Masternodes Voted",
  },
  locale: "en-US",
  name: "English",
  src: "en.json5",
};

void test("applyValidatorTermOverrides leaves dictionaries unchanged without a configured override", (): void => {
  const strings = {
    Validator: "Validator",
    Validators: "Validators",
  } as const;

  assert.deepEqual(applyValidatorTermOverridesForSelection(strings, language, null), strings);
});

void test("applyValidatorTermOverrides swaps validator labels from locale dict terms", (): void => {
  const strings = {
    Validator: "Validator",
    Validators: "Validators",
    "Validators Voted": "Validators Voted",
    validator: "validator",
    "{count, plural, one {# validator} other {# validators}}":
      "{count, plural, one {# validator} other {# validators}}",
    "{totalCount, plural, one {# validator} other {# validators}} ({enabledCount} active)":
      "{totalCount, plural, one {# validator} other {# validators}} ({enabledCount} active)",
  } as const;

  assert.deepEqual(
    applyValidatorTermOverridesForSelection(strings, language, {
      pluralKey: "masternodes",
      singularKey: "masternode",
    }),
    {
      Validator: "Masternode",
      Validators: "Masternodes",
      "Validators Voted": "Masternodes Voted",
      validator: "masternode",
      "{count, plural, one {# validator} other {# validators}}":
        "{count, plural, one {# masternode} other {# masternodes}}",
      "{totalCount, plural, one {# validator} other {# validators}} ({enabledCount} active)":
        "{totalCount, plural, one {# masternode} other {# masternodes}} ({enabledCount} active)",
    }
  );
});
