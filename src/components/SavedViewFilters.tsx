import type {
  CustomFieldDefinition,
  SavedViewCustomFieldPredicate,
  SavedViewDefinition,
  SavedViewEntityType,
  Tag,
} from "../api/types";

interface SavedViewFiltersProps {
  entityType: SavedViewEntityType;
  tags: Tag[];
  definitions: CustomFieldDefinition[];
  definition: SavedViewDefinition;
  onChange: (definition: SavedViewDefinition) => void;
}

const surfaceLabels: Record<SavedViewEntityType, string> = {
  contact: "contact",
  company: "company",
  opportunity: "pipeline",
};

const defaultOperator = (field: CustomFieldDefinition): SavedViewCustomFieldPredicate["operator"] => {
  if (field.fieldType === "text") return "contains";
  if (field.fieldType === "number") return "equals";
  if (field.fieldType === "date") return "on";
  return "is";
};

const operatorsFor = (field: CustomFieldDefinition) => {
  if (field.fieldType === "text") return [["contains", "Contains"], ["equals", "Equals"]] as const;
  if (field.fieldType === "number") return [["equals", "Equals"], ["greaterThanOrEqual", "At least"], ["lessThanOrEqual", "At most"]] as const;
  if (field.fieldType === "date") return [["on", "On"], ["before", "Before"], ["after", "After"]] as const;
  return [["is", "Is"]] as const;
};

/** Finite v2 filter editor; matching remains exclusively in the Rust command. */
export function SavedViewFilters({ entityType, tags, definitions, definition, onChange }: SavedViewFiltersProps) {
  const filter = definition.filter;
  const update = (next: Partial<SavedViewDefinition["filter"]>) => onChange({
    ...definition,
    filter: { ...filter, ...next },
  });
  const predicateFor = (field: CustomFieldDefinition) => filter.customFields.find((item) => item.definitionId === field.id);
  const replacePredicate = (field: CustomFieldDefinition, operator: string, rawValue: string) => {
    const remaining = filter.customFields.filter((item) => item.definitionId !== field.id);
    if (rawValue === "") {
      update({ customFields: remaining });
      return;
    }
    let predicate: SavedViewCustomFieldPredicate;
    if (field.fieldType === "text") predicate = { definitionId: field.id, fieldType: "text", operator: operator as "contains" | "equals", value: rawValue };
    else if (field.fieldType === "number") predicate = { definitionId: field.id, fieldType: "number", operator: operator as "equals" | "greaterThanOrEqual" | "lessThanOrEqual", value: Number(rawValue) };
    else if (field.fieldType === "date") predicate = { definitionId: field.id, fieldType: "date", operator: operator as "on" | "before" | "after", value: rawValue };
    else predicate = { definitionId: field.id, fieldType: "select", operator: "is", value: rawValue };
    update({ customFields: [...remaining, predicate] });
  };

  return <fieldset className="saved-view-filters">
    <legend>{surfaceLabels[entityType]} filters</legend>
    <label><input type="checkbox" checked={filter.includeArchived} onChange={(event) => update({ includeArchived: event.target.checked })} /> Include archived</label>
    <label>Tags (all must match)
      <select multiple aria-label="Tags (all must match)" value={filter.tagIdsAll} onChange={(event) => update({ tagIdsAll: Array.from(event.target.selectedOptions, (option) => option.value) })}>
        {tags.filter((tag) => !tag.archivedAt).map((tag) => <option key={tag.id} value={tag.id}>{tag.label}</option>)}
      </select>
    </label>
    {definitions.filter((field) => !field.archivedAt).map((field) => {
      const predicate = predicateFor(field);
      const operator = predicate?.operator ?? defaultOperator(field);
      const value = predicate?.value ?? "";
      return <div key={field.id} className="saved-view-filter">
        <label>{field.label} operator
          <select aria-label={`${field.label} operator`} value={operator} onChange={(event) => replacePredicate(field, event.target.value, String(value))}>
            {operatorsFor(field).map(([nextOperator, label]) => <option key={nextOperator} value={nextOperator}>{label}</option>)}
          </select>
        </label>
        <label>{field.label} value
          {field.fieldType === "select" ? <select aria-label={`${field.label} filter`} value={String(value)} onChange={(event) => replacePredicate(field, "is", event.target.value)}>
            <option value="">Any value</option>
            {field.options.map((option) => <option key={option.id} value={option.id}>{option.label}</option>)}
          </select> : <input
            aria-label={`${field.label} filter`}
            type={field.fieldType === "number" ? "number" : field.fieldType === "date" ? "date" : "text"}
            value={value}
            onChange={(event) => replacePredicate(field, operator, event.target.value)}
          />}
        </label>
      </div>;
    })}
  </fieldset>;
}
