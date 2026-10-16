import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { PRESET_TAB_OPTIONS } from "@/lib/preset-tabs";
import type { PrelaunchSite } from "@/types";

interface PrelaunchSitesSelectorProps {
  selectedUrls: string[];
  customSites?: PrelaunchSite[];
  onChange: (urls: string[]) => void;
  disabled?: boolean;
  idPrefix: string;
  t: (key: string) => string;
  placeholder?: string;
}

export function PrelaunchSitesSelector({
  selectedUrls,
  customSites = [],
  onChange,
  disabled = false,
  idPrefix,
  t,
  placeholder = "Select pre-launch sites",
}: PrelaunchSitesSelectorProps) {
  const options = [
    ...PRESET_TAB_OPTIONS.map((preset) => ({
      id: preset.url,
      label: t(`createProfile.presetTabs.${preset.key}`),
    })),
    ...customSites
      .filter(({ url }) => !PRESET_TAB_OPTIONS.some((preset) => preset.url === url))
      .map(({ name, url }) => ({ id: url, label: name })),
  ];

  const selectedLabels = options
    .filter((option) => selectedUrls.includes(option.id))
    .map((option) => option.label);

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="outline"
          disabled={disabled || options.length === 0}
          className="w-full justify-between font-normal"
        >
          <span className="truncate">
            {selectedLabels.length > 0
              ? selectedLabels.join(", ")
              : placeholder}
          </span>
          <span className="ml-2 shrink-0 text-xs text-muted-foreground">
            {selectedLabels.length}
          </span>
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-[min(28rem,calc(100vw-2rem))] p-2">
        <div className="max-h-64 space-y-1 overflow-y-auto">
          {options.map((option) => {
            const id = `${idPrefix}-${encodeURIComponent(option.id)}`;
            return (
              <label
                key={option.id}
                htmlFor={id}
                className="flex cursor-pointer items-center gap-3 rounded px-2 py-2 text-sm hover:bg-accent"
              >
                <Checkbox
                  id={id}
                  disabled={disabled}
                  checked={selectedUrls.includes(option.id)}
                  onCheckedChange={(checked) => {
                    onChange(
                      checked === true
                        ? Array.from(new Set([...selectedUrls, option.id]))
                        : selectedUrls.filter((url) => url !== option.id),
                    );
                  }}
                />
                <span className="min-w-0 flex-1 truncate" title={option.id}>
                  {option.label}
                </span>
              </label>
            );
          })}
        </div>
      </PopoverContent>
    </Popover>
  );
}
