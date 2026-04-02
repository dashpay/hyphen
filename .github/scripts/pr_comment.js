/*!
 * Copyright (c) 2026-present, Kittywhiskers Van Gogh
 * SPDX-License-Identifier: MIT
 * See the accompanying file LICENSE or https://opensource.org/license/MIT
 */

// @ts-check

const BASE_BRANCH = "develop";
const COMMENT_TAG = "<!-- pr-status-bot -->";
const MERGEABLE_RETRIES = 5;
const MERGEABLE_DELAY_MS = 3000;

const WORKFLOW_CHAIN = [
  { name: "Run linters" },
  { name: "Rust" },
  { name: "TypeScript" },
  { name: "Test hyphen-daemon" },
  { name: "Test @repo/hyphen-webui" },
];

/**
 * @param {{ github: import("@actions/github").getOctokit, context: import("@actions/github").context }} params
 */
module.exports = async ({ github, context }) => {
  const owner = context.repo.owner;
  const repo = context.repo.repo;

  const { data: pulls } = await github.rest.pulls.list({
    owner,
    repo,
    state: "open",
    base: BASE_BRANCH,
    per_page: 100,
  });

  const fileCache = await buildFileCache({ github, owner, repo, pulls });

  for (const pr of pulls) {
    const body = await buildComment({ github, owner, repo, pr, pulls, fileCache });
    await upsertComment({ github, owner, repo, prNumber: pr.number, body });
    console.log(`PR #${pr.number}: comment updated`);
  }
};

/**
 * @param {{ github: any, owner: string, repo: string, pulls: any[] }} params
 * @returns {Promise<Map<number, string[]>>}
 */
async function buildFileCache({ github, owner, repo, pulls }) {
  const cache = new Map();
  const results = await Promise.all(
    pulls.map((pr) =>
      github.rest.pulls.listFiles({
        owner,
        repo,
        pull_number: pr.number,
        per_page: 100,
      }).then((res) => ({ number: pr.number, files: res.data.map((f) => f.filename) }))
    )
  );
  for (const entry of results) {
    cache.set(entry.number, entry.files);
  }
  return cache;
}

/**
 * @param {number} ms
 * @returns {Promise<void>}
 */
function sleep(ms) {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

/**
 * @param {{ github: any, owner: string, repo: string, prNumber: number }} params
 * @returns {Promise<string>}
 */
async function getMergeableState({ github, owner, repo, prNumber }) {
  for (let i = 0; i < MERGEABLE_RETRIES; i++) {
    const { data: detail } = await github.rest.pulls.get({
      owner,
      repo,
      pull_number: prNumber,
    });
    if (detail.mergeable_state !== "unknown") {
      return detail.mergeable_state;
    }
    await sleep(MERGEABLE_DELAY_MS);
  }
  return "unknown";
}

/**
 * @param {{ github: any, owner: string, repo: string, pr: any, pulls: any[], fileCache: Map<number, string[]> }} params
 * @returns {Promise<string>}
 */
async function buildComment({ github, owner, repo, pr, pulls, fileCache }) {
  const sha = pr.head.sha;

  const mergeableState = await getMergeableState({ github, owner, repo, prNumber: pr.number });
  const isDirty = mergeableState === "dirty";

  const ourFiles = new Set(fileCache.get(pr.number) || []);
  const potentialConflicts = [];

  for (const other of pulls) {
    if (other.number === pr.number) {
      continue;
    }
    const otherFiles = fileCache.get(other.number) || [];
    const overlap = otherFiles.filter((f) => ourFiles.has(f));
    if (overlap.length > 0) {
      potentialConflicts.push({ url: other.html_url });
    }
  }

  let banner;
  if (isDirty) {
    banner = "> [!CAUTION]\n> This pull request conflicts with the base branch. Please rebase and force-push.";
  } else if (potentialConflicts.length > 0) {
    banner =
      "> [!WARNING]\n> This pull request may have conflicts, please coordinate with the authors of these pull requests.";
  } else {
    banner = "> [!NOTE]\n> This pull request has no conflicts! 🎊 🎉 🎊";
  }

  let conflictSection = "";
  if (potentialConflicts.length > 0) {
    conflictSection = "## Potential conflicts\n\n";
    for (const c of potentialConflicts) {
      conflictSection += `* ${c.url}\n`;
    }
    conflictSection += "\n";
  }

  const { data: runs } = await github.rest.actions.listWorkflowRunsForRepo({
    owner,
    repo,
    head_sha: sha,
    per_page: 100,
  });

  let actionsSection = "## Actions\n\n";
  actionsSection += "| Workflow | Status | Link |\n";
  actionsSection += "|:--------:|:------:|:----:|\n";

  let chainBroken = false;
  for (const wf of WORKFLOW_CHAIN) {
    const run = runs.workflow_runs.find((r) => r.name === wf.name);
    if (chainBroken) {
      actionsSection += `| ${wf.name} | ⏩ Skipped | - |\n`;
    } else if (run) {
      let status;
      if (run.conclusion === "success") {
        status = "🟢 Pass";
      } else if (run.conclusion === "failure") {
        status = "🔴 Fail";
        chainBroken = true;
      } else if (run.conclusion === "cancelled") {
        status = "⏩ Skipped";
        chainBroken = true;
      } else if (run.status === "in_progress") {
        status = "🟡 In Progress";
      } else if (run.status === "queued" || run.status === "waiting") {
        status = "⌛ Waiting";
      } else {
        status = "⌛ Waiting";
      }
      const link = `[Run #${run.run_number}](${run.html_url})`;
      actionsSection += `| ${wf.name} | ${status} | ${link} |\n`;
    } else {
      actionsSection += `| ${wf.name} | ⌛ Waiting | - |\n`;
    }
  }

  return `${COMMENT_TAG}\n${banner}\n\n${conflictSection}\n${actionsSection}`;
}

/**
 * @param {{ github: any, owner: string, repo: string, prNumber: number, body: string }} params
 */
async function upsertComment({ github, owner, repo, prNumber, body }) {
  const { data: comments } = await github.rest.issues.listComments({
    owner,
    repo,
    issue_number: prNumber,
    per_page: 100,
  });

  const existing = comments.find((c) => c.body.startsWith(COMMENT_TAG));

  if (existing) {
    await github.rest.issues.updateComment({
      owner,
      repo,
      comment_id: existing.id,
      body,
    });
  } else {
    await github.rest.issues.createComment({
      owner,
      repo,
      issue_number: prNumber,
      body,
    });
  }
}
