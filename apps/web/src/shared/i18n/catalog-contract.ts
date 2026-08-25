const pluralSuffix = /_(zero|one|two|few|many|other)$/u;
const placeholderPattern = /\{\{\s*([\w.]+)(?:\s*,[^}]*)?\s*\}\}/gu;

export type MessageCatalog = Readonly<Record<string, string>>;

export type CatalogContractFailure = {
  readonly code: 'i18n.missing_canonical_key' | 'i18n.placeholder_mismatch';
  readonly detail: string;
  readonly key: string;
  readonly language: string;
};

export function canonicalMessageKey(key: string): string {
  return key.replace(pluralSuffix, '');
}

export function validateCatalogContract(
  referenceLanguage: string,
  reference: MessageCatalog,
  catalogs: Readonly<Record<string, MessageCatalog>>,
): readonly CatalogContractFailure[] {
  const referenceIndex = indexCatalog(reference);
  const failures: CatalogContractFailure[] = [];

  for (const [language, catalog] of Object.entries(catalogs)) {
    const catalogIndex = indexCatalog(catalog);
    for (const [key, placeholders] of referenceIndex) {
      const candidate = catalogIndex.get(key);
      if (candidate === undefined) {
        failures.push({
          code: 'i18n.missing_canonical_key',
          detail: referenceLanguage,
          key,
          language,
        });
        continue;
      }
      if (!sameSet(placeholders, candidate)) {
        failures.push({
          code: 'i18n.placeholder_mismatch',
          detail: `${formatSet(placeholders)} != ${formatSet(candidate)}`,
          key,
          language,
        });
      }
    }
  }

  return failures;
}

function indexCatalog(catalog: MessageCatalog): ReadonlyMap<string, ReadonlySet<string>> {
  const index = new Map<string, Set<string>>();
  for (const [key, message] of Object.entries(catalog)) {
    const canonical = canonicalMessageKey(key);
    const placeholders = index.get(canonical) ?? new Set<string>();
    for (const match of message.matchAll(placeholderPattern)) {
      const name = match[1];
      if (name !== undefined) {
        placeholders.add(name);
      }
    }
    index.set(canonical, placeholders);
  }
  return index;
}

function sameSet(left: ReadonlySet<string>, right: ReadonlySet<string>): boolean {
  return left.size === right.size && [...left].every((item) => right.has(item));
}

function formatSet(values: ReadonlySet<string>): string {
  return [...values].sort().join(',');
}
