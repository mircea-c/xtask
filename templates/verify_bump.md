### verify-bump {{ status }}: `{{ previous }}` -> `{{ bumped }}`

Compared against `{{ base }}`.

| Check | Result |
|---|---|
| Changed files | {{ report.changed_files }} |
| Manifests | {{ report.manifests_checked }} checked, {{ report.fields() }} version field(s) bumped in {{ report.manifests.len() }} |
| Lock files | {{ report.locks.len() }} checked, {{ report.lock_packages() }} package(s) bumped |
| Problems | {{ report.problems.len() }} |
{%- if !report.problems.is_empty() %}

#### Problems

{{ backticks }}
{{ problems }}
{{ backticks }}
{%- endif %}
{%- if !report.manifests.is_empty() %}

<details><summary>Manifest fields ({{ report.fields() }})</summary>

| File | Field | Before | After |
|---|---|---|---|
{%- for (file, fields) in report.manifests %}
{%- for (field, (old, new)) in fields %}
| `{{ file }}` | `{{ field }}` | `{{ old }}` | `{{ new }}` |
{%- endfor %}
{%- endfor %}

</details>
{%- endif %}
{%- if !report.locks.is_empty() %}

<details><summary>Lock packages ({{ report.lock_packages() }})</summary>
{% for (file, summary) in report.locks %}
- `{{ file }}`, {{ summary.bumped.len() }} of {{ summary.packages }}: {% for name in summary.bumped %}`{{ name }}`{% if !loop.last %}, {% endif %}{% endfor %}
{%- endfor %}

</details>
{%- endif %}

