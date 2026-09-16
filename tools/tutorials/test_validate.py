import json
import tempfile
import unittest
from pathlib import Path

from .validate import ValidationError, load, validate_document, validate_metadata


ROOT = Path(__file__).parent
METADATA = load(ROOT / "command_metadata_v1.json")
SCHEMA = load(ROOT / "schema/v1/tutorial.schema.json")


def document(command):
    return {
        "schema_version": 1,
        "tutorial_id": "test",
        "title": "Test tutorial",
        "steps": [{
            "id": "step-one",
            "command": command,
            "expect": {"exit_code": 0, "stdout_shape": "json"},
            "network": "denied",
            "credentials": "none",
        }],
    }


class TutorialValidatorTests(unittest.TestCase):
    def test_valid_contract(self):
        validate_document(document(["asb", "capabilities", "--format", "json"]), METADATA)

    def test_unknown_option_fails(self):
        with self.assertRaisesRegex(ValidationError, "do not match"):
            validate_document(document(["asb", "capabilities", "--json"]), METADATA)

    def test_reordered_option_fails(self):
        with self.assertRaises(ValidationError):
            validate_document(document(["asb", "capabilities", "json", "--format"]), METADATA)

    def test_secret_and_network_contracts_fail(self):
        with self.assertRaises(ValidationError):
            validate_document(document(["asb", "report", "token.json"]), METADATA)
        bad = document(["asb", "doctor"])
        bad["steps"][0]["network"] = "allowed"
        with self.assertRaises(ValidationError):
            validate_document(bad, METADATA)
        bad["steps"][0]["network"] = ["denied"]
        with self.assertRaises(ValidationError):
            validate_document(bad, METADATA)
        bad["steps"][0]["network"] = "denied"
        bad["steps"][0]["credentials"] = {"value": "none"}
        with self.assertRaises(ValidationError):
            validate_document(bad, METADATA)

    def test_network_argument_cannot_be_smuggled_into_command(self):
        with self.assertRaises(ValidationError):
            validate_document(document(["asb", "doctor", "--network", "allowed"]), METADATA)
        with self.assertRaises(ValidationError):
            validate_document(document(["asb", "report", "https://example.invalid/result.json"]), METADATA)

    def test_unknown_field_fails(self):
        bad = document(["asb", "doctor"])
        bad["steps"][0]["unexpected"] = True
        with self.assertRaises(ValidationError):
            validate_document(bad, METADATA)

    def test_metadata_option_removal_is_detected(self):
        metadata = json.loads(json.dumps(METADATA))
        metadata["commands"]["capabilities"]["forms"] = [[]]
        with self.assertRaises(ValidationError):
            validate_document(document(["asb", "capabilities", "--format", "json"]), metadata)

    def test_metadata_is_validated_before_use(self):
        metadata = json.loads(json.dumps(METADATA))
        metadata["commands"]["doctor"]["forms"] = "not-an-array"
        with self.assertRaises(ValidationError):
            validate_metadata(metadata)
        metadata = json.loads(json.dumps(METADATA))
        metadata["commands"]["capabilities"]["forms"] = [["--renamed", "json"]]
        with self.assertRaises(ValidationError):
            validate_metadata(metadata)

    def test_path_operands_cannot_be_options(self):
        with self.assertRaises(ValidationError):
            validate_document(document(["asb", "compare", "--unknown", "run.json"]), METADATA)

    def test_schema_and_validator_contract_fields_stay_in_parity(self):
        self.assertEqual(
            set(SCHEMA["required"]), {"schema_version", "tutorial_id", "title", "steps"}
        )
        self.assertEqual(
            set(SCHEMA["properties"]), {"schema_version", "tutorial_id", "title", "steps"}
        )
        step = SCHEMA["$defs"]["step"]
        self.assertEqual(set(step["required"]), {"id", "command", "expect"})
        self.assertEqual(
            set(step["properties"]),
            {"id", "command", "expect", "references", "network", "credentials"},
        )
        self.assertEqual(SCHEMA["properties"]["schema_version"]["const"], 1)
        self.assertEqual(step["properties"]["network"]["const"], "denied")
        self.assertEqual(step["properties"]["credentials"]["const"], "none")
        self.assertFalse(SCHEMA["additionalProperties"])
        self.assertFalse(step["additionalProperties"])
        self.assertEqual(step["properties"]["command"]["maxItems"], 16)
        self.assertEqual(step["properties"]["references"]["maxItems"], 16)

    def test_file_loader_rejects_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.json"
            link = Path(directory) / "link.json"
            source.write_text("{}", encoding="utf-8")
            link.symlink_to(source)
            with self.assertRaises(ValidationError):
                load(link)


if __name__ == "__main__":
    unittest.main()
