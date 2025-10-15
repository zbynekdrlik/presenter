## ADDED Requirements
### Requirement: Bible list header mirrors worship libraries
Operators MUST see the Bible translations list presented with the same header structure and layout used for the worship Libraries list.
#### Scenario: Bible panel renders in operator view
- **GIVEN** an operator loads `/ui/operator?view=bible`
- **WHEN** the Bible sidebar is rendered
- **THEN** the translations section appears first in the sidebar with the header text `Bibles (N)` where `N` equals the number of available translations
- **AND** the header uses the `operator__group-header` + `operator__group-controls` structure without subordinate helper copy, matching the worship Libraries component spacing and typography

### Requirement: Bible header controls reuse library affordances
Operators MUST have parity between Bible and library controls for surfacing dashboards and starting import flows.
#### Scenario: Operator surfaces Bibles on the dashboard
- **GIVEN** the Bible sidebar header shows a count button next to `Bibles`
- **WHEN** the operator activates the count control
- **THEN** the existing dashboard surfacing picker opens with the list of Bible translations, identical to the experience provided for worship Libraries
#### Scenario: Operator launches Bible import
- **GIVEN** the Bible sidebar header shows a `+` control
- **WHEN** the operator clicks `+`
- **THEN** the Bible import/add flow opens using the same pathway invoked by the current Bible add action

### Requirement: Bible translations allow inline management
Operators MUST be able to manage individual Bible translations from the operator view without leaving the panel.
#### Scenario: Operator edits a Bible translation
- **GIVEN** a Bible translation row in the sidebar
- **WHEN** the operator activates the overflow menu for that translation
- **THEN** a modal opens allowing the operator to rename the Bible and adjust its language, and saving the form updates the list immediately
#### Scenario: Operator deletes a Bible translation
- **GIVEN** the Bible translation edit modal is open for a translation
- **WHEN** the operator confirms deletion
- **THEN** the translation and its passages are removed, the header count updates, and the dashboard pin state is recalculated automatically
