#!/usr/bin/env python3
"""Render a completed screen's actual peaks; separate metrics and axis scales."""
import argparse
import json
from pathlib import Path
import statistics

import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('summary', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = json.loads(args.summary.read_text())
    assert result['valid'] and result['stage'] == 'screen' and not result['qualification']
    source = [r for r in result['rows'] if r['group'] == 'latin']
    assert len(source) == 16
    modes = ('idle', 'names', 'stat', 'scan')
    labels = ('Idle', 'Names', 'Names + stat', 'v1 scan')
    colors = ('#8c8c8c', '#4c78a8', '#f58518', '#54a24b')
    fig, axes = plt.subplots(1, 2, figsize=(10.5, 4.4))
    metrics = (('kernel_memory_peak_bytes', 2**30, 'Whole worker peak (GiB)'),
               ('peak_rss_kib', 1024, 'Command process peak RSS (MiB)'))
    for ax, (key, divisor, title) in zip(axes, metrics):
        groups = [[r[key] / divisor for r in source if r['mode'] == mode] for mode in modes]
        assert all(len(group) == 4 for group in groups)
        medians = [statistics.median(group) for group in groups]
        ax.bar(range(4), medians, color=colors, alpha=.62, width=.65, zorder=2)
        for x, group in enumerate(groups):
            ax.scatter([x - .09 + .06 * n for n in range(4)], group,
                       color='#222222', s=20, zorder=3)
            ax.text(x, max(group) + max(medians)*.06, f'{medians[x]:.2f}',
                    ha='center', fontsize=9)
        ax.set_xticks(range(4), labels)
        ax.set_ylim(0, max(max(group) for group in groups) * 1.25)
        ax.set_title(title, fontsize=11)
        ax.grid(axis='y', alpha=.2, zorder=0)
        ax.spines[['top', 'right']].set_visible(False)
    fig.suptitle('Memory in the matched metadata experiment', fontsize=14)
    fig.text(.5, .075, '1,000,000 files / 1,004,499 entries; four Williams orders; bars = medians, points = runs.',
             ha='center', fontsize=9)
    fig.text(.5, .025, 'Different axis scales. Worker peaks include wrappers and OS charges; process RSS is a separate measure.',
             ha='center', fontsize=8)
    fig.tight_layout(rect=(0, .13, 1, .93))
    for suffix in ('.png', '.svg'):
        output = args.output.with_suffix(suffix)
        fig.savefig(output, dpi=180)
        if suffix == '.svg':
            output.write_text('\n'.join(line.rstrip() for line in output.read_text().splitlines()) + '\n')
    plt.close(fig)


if __name__ == '__main__':
    main()
