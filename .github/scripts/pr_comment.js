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

const CI_WORKFLOW = "CI";
const JOB_CHAIN = [
  { name: "Build container" },
  { name: "Run linters" },
  { name: "Rust" },
  { name: "TypeScript" },
  { name: "Test hyphen-daemon" },
  { name: "Test hyphen-db" },
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
  const ciRuns = runs.workflow_runs.filter((r) => r.name === CI_WORKFLOW);
  // Fork PR runs list no associated pull requests, so also match on the head repository and branch.
  const run = ciRuns.find(
    (r) =>
      r.event === "pull_request" &&
      (r.pull_requests.some((p) => p.number === pr.number) ||
        (r.head_repository?.full_name === pr.head.repo?.full_name && r.head_branch === pr.head.ref))
  );
  const jobs = run
    ? await github.paginate(github.rest.actions.listJobsForWorkflowRun, {
        owner,
        repo,
        run_id: run.id,
        per_page: 100,
      })
    : [];

  let actionsSection = "## Actions\n\n";
  actionsSection += "| Job | Status | Link |\n";
  actionsSection += "|:---:|:------:|:----:|\n";

  // A job whose dependency failed is itself reported as skipped, so each row
  // shows only its own jobs.
  for (const step of JOB_CHAIN) {
    // Jobs from reusable workflows are named "<caller> / <callee>".
    const matched = jobs.filter((j) => j.name === step.name || j.name.startsWith(`${step.name} / `));
    if (matched.length === 0) {
      actionsSection += `| ${step.name} | ⌛ Waiting | - |\n`;
      continue;
    }
    let status;
    if (matched.some((j) => j.conclusion === "failure" || j.conclusion === "timed_out")) {
      status = "🔴 Fail";
    } else if (matched.some((j) => j.conclusion === "cancelled" || j.conclusion === "skipped")) {
      status = "⏩ Skipped";
    } else if (matched.every((j) => j.conclusion === "success")) {
      status = "🟢 Pass";
    } else if (matched.some((j) => j.status === "in_progress")) {
      status = "🟡 In Progress";
    } else {
      status = "⌛ Waiting";
    }
    actionsSection += `| ${step.name} | ${status} | [Job](${matched[0].html_url}) |\n`;
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
