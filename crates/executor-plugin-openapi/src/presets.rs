//! Google Discovery and Microsoft Graph catalog presets.
//!
//! Graph's published `OpenAPI` monolith is ~43 MB. This adapter never fetches it.
//! Callers pass a sliced spec plus a preset id so `resolve_tools` keeps matching
//! path prefixes / exact paths.

use executor_core::PluginError;
use serde_json::{Map, Value, json};

/// Microsoft Graph v1.0 `OpenAPI` monolith (never fetched by this plugin).
pub const MICROSOFT_GRAPH_OPENAPI_URL: &str = "https://raw.githubusercontent.com/microsoftgraph/msgraph-metadata/master/openapi/v1.0/openapi.yaml";
/// Graph HTTP root.
pub const MICROSOFT_GRAPH_BASE_URL: &str = "https://graph.microsoft.com/v1.0";
/// Azure AD authorize URL.
pub const MICROSOFT_AUTHORIZATION_URL: &str =
    "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
/// Azure AD token URL.
pub const MICROSOFT_TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
/// Google OAuth authorize URL.
pub const GOOGLE_AUTHORIZATION_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
/// Google OAuth token URL.
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// One Google Discovery preset.
#[derive(Clone, Copy, Debug)]
pub struct GooglePreset {
    /// Preset id (`google-gmail`).
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// One-line summary.
    pub summary: &'static str,
    /// Discovery document URL.
    pub url: &'static str,
}

/// One Microsoft Graph scope slice.
#[derive(Clone, Copy, Debug)]
pub struct GraphScopePreset {
    /// Preset id (`mail`).
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// One-line summary.
    pub summary: &'static str,
    /// OAuth scopes.
    pub scopes: &'static [&'static str],
    /// Path-prefix keep list.
    pub path_prefixes: &'static [&'static str],
    /// Exact path keep list.
    pub exact_paths: &'static [&'static str],
}

/// Google Discovery presets (local boot catalog).
pub const GOOGLE_PRESETS: &[GooglePreset] = &[
    GooglePreset {
        id: "google-calendar",
        name: "Google Calendar",
        summary: "Calendars, events, ACLs, and scheduling.",
        url: "https://www.googleapis.com/discovery/v1/apis/calendar/v3/rest",
    },
    GooglePreset {
        id: "google-meet",
        name: "Google Meet",
        summary: "Meeting spaces, conference records, participants, recordings, and transcripts.",
        url: "https://meet.googleapis.com/$discovery/rest?version=v2",
    },
    GooglePreset {
        id: "google-gmail",
        name: "Gmail",
        summary: "Messages, threads, labels, and drafts.",
        url: "https://www.googleapis.com/discovery/v1/apis/gmail/v1/rest",
    },
    GooglePreset {
        id: "google-sheets",
        name: "Google Sheets",
        summary: "Spreadsheets, values, ranges, and formatting.",
        url: "https://www.googleapis.com/discovery/v1/apis/sheets/v4/rest",
    },
    GooglePreset {
        id: "google-drive",
        name: "Google Drive",
        summary: "Files, folders, permissions, and shared drives.",
        url: "https://www.googleapis.com/discovery/v1/apis/drive/v3/rest",
    },
    GooglePreset {
        id: "google-docs",
        name: "Google Docs",
        summary: "Documents, structural edits, and formatting.",
        url: "https://www.googleapis.com/discovery/v1/apis/docs/v1/rest",
    },
    GooglePreset {
        id: "google-slides",
        name: "Google Slides",
        summary: "Presentations, slides, page elements, and deck updates.",
        url: "https://www.googleapis.com/discovery/v1/apis/slides/v1/rest",
    },
    GooglePreset {
        id: "google-forms",
        name: "Google Forms",
        summary: "Forms, questions, responses, and quizzes.",
        url: "https://forms.googleapis.com/$discovery/rest?version=v1",
    },
    GooglePreset {
        id: "google-tasks",
        name: "Google Tasks",
        summary: "Task lists, task items, notes, and due dates.",
        url: "https://www.googleapis.com/discovery/v1/apis/tasks/v1/rest",
    },
    GooglePreset {
        id: "google-people",
        name: "Google People",
        summary: "Contacts, profiles, directory people, and contact groups.",
        url: "https://www.googleapis.com/discovery/v1/apis/people/v1/rest",
    },
    GooglePreset {
        id: "google-photos-library",
        name: "Google Photos Library",
        summary: "Albums, uploads, and app-created media through Google Photos.",
        url: "https://www.googleapis.com/discovery/v1/apis/photoslibrary/v1/rest",
    },
    GooglePreset {
        id: "google-photos-picker",
        name: "Google Photos Picker",
        summary: "Picker sessions and user-selected Google Photos media items.",
        url: "https://photospicker.googleapis.com/$discovery/rest?version=v1",
    },
    GooglePreset {
        id: "google-chat",
        name: "Google Chat",
        summary: "Spaces, messages, members, reactions, and chat workflows.",
        url: "https://www.googleapis.com/discovery/v1/apis/chat/v1/rest",
    },
    GooglePreset {
        id: "google-keep",
        name: "Google Keep",
        summary: "Create, list, delete, and share notes; download attachments.",
        url: "https://keep.googleapis.com/$discovery/rest?version=v1",
    },
    GooglePreset {
        id: "google-youtube-data",
        name: "YouTube Data",
        summary: "Channels, playlists, videos, comments, and uploads.",
        url: "https://www.googleapis.com/discovery/v1/apis/youtube/v3/rest",
    },
    GooglePreset {
        id: "google-search-console",
        name: "Google Search Console",
        summary: "Sites, sitemaps, URL inspection, and search performance.",
        url: "https://www.googleapis.com/discovery/v1/apis/searchconsole/v1/rest",
    },
    GooglePreset {
        id: "google-classroom",
        name: "Google Classroom",
        summary: "Courses, rosters, coursework, and grading.",
        url: "https://www.googleapis.com/discovery/v1/apis/classroom/v1/rest",
    },
    GooglePreset {
        id: "google-admin-directory",
        name: "Google Admin Directory",
        summary: "Users, groups, org units, roles, and domain resources.",
        url: "https://admin.googleapis.com/$discovery/rest?version=directory_v1",
    },
    GooglePreset {
        id: "google-admin-reports",
        name: "Google Admin Reports",
        summary: "Audit events, usage reports, and admin activity logs.",
        url: "https://admin.googleapis.com/$discovery/rest?version=reports_v1",
    },
    GooglePreset {
        id: "google-apps-script",
        name: "Google Apps Script",
        summary: "Projects, deployments, versions, processes, and metrics.",
        url: "https://www.googleapis.com/discovery/v1/apis/script/v1/rest",
    },
    GooglePreset {
        id: "google-bigquery",
        name: "Google BigQuery",
        summary: "Datasets, tables, jobs, and analytical queries.",
        url: "https://www.googleapis.com/discovery/v1/apis/bigquery/v2/rest",
    },
    GooglePreset {
        id: "google-cloud-resource-manager",
        name: "Google Cloud Resource Manager",
        summary: "Projects, folders, organizations, and IAM hierarchy.",
        url: "https://cloudresourcemanager.googleapis.com/$discovery/rest?version=v3",
    },
];

/// Microsoft Graph scope presets.
pub const GRAPH_SCOPE_PRESETS: &[GraphScopePreset] = &[
    GraphScopePreset {
        id: "profile",
        name: "Profile",
        summary: "Signed-in user profile and photo.",
        scopes: &["User.Read"],
        path_prefixes: &[],
        exact_paths: &["/me", "/me/photo", "/me/photo/$value"],
    },
    GraphScopePreset {
        id: "me-surface",
        name: "My Graph Operations",
        summary: "All operation groups rooted under /me.",
        scopes: &["User.Read"],
        path_prefixes: &["/me"],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "mail",
        name: "Outlook Mail",
        summary: "Messages, folders, attachments, settings, and send mail.",
        scopes: &["Mail.ReadWrite", "Mail.Send", "MailboxSettings.ReadWrite"],
        path_prefixes: &[
            "/me/messages",
            "/me/mailFolders",
            "/me/sendMail",
            "/me/getMailTips",
            "/me/inferenceClassification",
            "/me/mailboxSettings",
            "/me/outlook",
            "/users/{user-id}/messages",
            "/users/{user-id}/mailFolders",
            "/users/{user-id}/sendMail",
            "/users/{user-id}/outlook",
        ],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "calendar",
        name: "Outlook Calendar",
        summary: "Calendars, events, and scheduling.",
        scopes: &["Calendars.ReadWrite"],
        path_prefixes: &[
            "/me/calendar",
            "/me/calendars",
            "/me/calendarGroups",
            "/me/calendarView",
            "/me/events",
            "/me/findMeetingTimes",
            "/me/reminderView",
            "/users/{user-id}/calendar",
            "/users/{user-id}/calendars",
            "/users/{user-id}/calendarGroups",
            "/users/{user-id}/calendarView",
            "/users/{user-id}/events",
            "/users/{user-id}/findMeetingTimes",
            "/users/{user-id}/reminderView",
        ],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "contacts",
        name: "Outlook Contacts",
        summary: "Contacts, contact folders, and people suggestions.",
        scopes: &["Contacts.ReadWrite", "People.Read.All"],
        path_prefixes: &[
            "/me/contacts",
            "/me/contactFolders",
            "/me/people",
            "/users/{user-id}/contacts",
            "/users/{user-id}/contactFolders",
            "/users/{user-id}/people",
        ],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "tasks",
        name: "To Do Tasks",
        summary: "Task lists, tasks, and checklist items.",
        scopes: &["Tasks.ReadWrite"],
        path_prefixes: &["/me/todo", "/users/{user-id}/todo"],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "planner",
        name: "Planner",
        summary: "Plans, buckets, tasks, assignments, and Planner user data.",
        scopes: &["Tasks.ReadWrite"],
        path_prefixes: &[
            "/planner",
            "/me/planner",
            "/users/{user-id}/planner",
            "/groups/{group-id}/planner",
        ],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "files",
        name: "OneDrive Files",
        summary: "Drives, files, folders, sharing links, and permissions.",
        scopes: &["Files.ReadWrite.All", "Sites.ReadWrite.All"],
        path_prefixes: &[
            "/me/drive",
            "/me/drives",
            "/me/followedSites",
            "/users/{user-id}/drive",
            "/users/{user-id}/drives",
            "/groups/{group-id}/drive",
            "/groups/{group-id}/drives",
            "/drives",
            "/shares",
        ],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "excel",
        name: "Excel Workbooks",
        summary: "Workbook tables, worksheets, ranges, charts, and sessions.",
        scopes: &["Files.ReadWrite.All"],
        path_prefixes: &[
            "/me/drive/items/{driveItem-id}/workbook",
            "/users/{user-id}/drive/items/{driveItem-id}/workbook",
            "/groups/{group-id}/drive/items/{driveItem-id}/workbook",
            "/drives/{drive-id}/items/{driveItem-id}/workbook",
        ],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "sites",
        name: "SharePoint Sites",
        summary: "Sites, lists, pages, columns, content types, and stores.",
        scopes: &["Sites.ReadWrite.All"],
        path_prefixes: &["/sites"],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "onenote",
        name: "OneNote",
        summary: "Notebooks, sections, pages, and page content.",
        scopes: &["Notes.ReadWrite"],
        path_prefixes: &[
            "/me/onenote",
            "/users/{user-id}/onenote",
            "/groups/{group-id}/onenote",
            "/sites/{site-id}/onenote",
        ],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "teams-chat",
        name: "Teams Chats",
        summary: "Chats, chat messages, installed apps, and members.",
        scopes: &["Chat.ReadWrite"],
        path_prefixes: &["/me/chats", "/chats"],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "teams-channels",
        name: "Teams Channels",
        summary: "Teams, channels, channel messages, replies, and joined teams.",
        scopes: &[
            "Team.ReadBasic.All",
            "Channel.ReadBasic.All",
            "ChannelMessage.Read.All",
            "ChannelMessage.Send",
        ],
        path_prefixes: &[
            "/me/joinedTeams",
            "/groups/{group-id}/team",
            "/teams",
            "/teamwork",
            "/teamsTemplates",
        ],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "meetings-calls",
        name: "Meetings and Calls",
        summary: "Online meetings, calls, call records, and communications APIs.",
        scopes: &["OnlineMeetings.ReadWrite"],
        path_prefixes: &[
            "/communications",
            "/me/onlineMeetings",
            "/users/{user-id}/onlineMeetings",
        ],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "users",
        name: "Users",
        summary: "User objects plus user-scoped Graph operations.",
        scopes: &["User.ReadWrite.All", "Directory.Read.All"],
        path_prefixes: &["/users", "/users(userPrincipalName='{userPrincipalName}')"],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "groups",
        name: "Groups",
        summary: "Groups, settings, lifecycle policies, and group-scoped operations.",
        scopes: &["Group.ReadWrite.All", "Directory.Read.All"],
        path_prefixes: &[
            "/groups",
            "/groups(uniqueName='{uniqueName}')",
            "/groupSettings",
            "/groupSettingTemplates",
            "/groupLifecyclePolicies",
        ],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "directory",
        name: "Directory",
        summary: "Directory roles, objects, contacts, contracts, and invitations.",
        scopes: &["Directory.Read.All"],
        path_prefixes: &[
            "/contacts",
            "/contracts",
            "/directory",
            "/directoryObjects",
            "/directoryRoles",
            "/directoryRoles(roleTemplateId='{roleTemplateId}')",
            "/directoryRoleTemplates",
            "/invitations",
            "/scopedRoleMemberships",
        ],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "applications",
        name: "Applications",
        summary: "Applications, service principals, app templates, catalogs, and grants.",
        scopes: &[
            "Application.ReadWrite.All",
            "AppRoleAssignment.ReadWrite.All",
        ],
        path_prefixes: &[
            "/applications",
            "/applications(appId='{appId}')",
            "/applications(uniqueName='{uniqueName}')",
            "/applicationTemplates",
            "/appCatalogs",
            "/oauth2PermissionGrants",
            "/permissionGrants",
            "/servicePrincipals",
            "/servicePrincipals(appId='{appId}')",
        ],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "identity",
        name: "Identity and Governance",
        summary: "Identity, governance, policies, access reviews, roles, and providers.",
        scopes: &[
            "Policy.ReadWrite.ConditionalAccess",
            "RoleManagement.Read.Directory",
        ],
        path_prefixes: &[
            "/agreementAcceptances",
            "/agreements",
            "/authenticationMethodConfigurations",
            "/authenticationMethodsPolicy",
            "/certificateBasedAuthConfiguration",
            "/identity",
            "/identityGovernance",
            "/identityProviders",
            "/identityProtection",
            "/policies",
            "/roleManagement",
        ],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "admin-reports",
        name: "Admin and Reports",
        summary: "Admin centers, audit logs, domains, reports, organization, and tenants.",
        scopes: &["AuditLog.Read.All", "Reports.Read.All"],
        path_prefixes: &[
            "/admin",
            "/auditLogs",
            "/domains",
            "/domainDnsRecords",
            "/organization",
            "/reports",
            "/subscribedSkus",
            "/tenantRelationships",
        ],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "security-compliance",
        name: "Security and Compliance",
        summary: "Security, compliance, privacy, information protection, and data policy.",
        scopes: &["SecurityEvents.Read.All"],
        path_prefixes: &[
            "/compliance",
            "/dataPolicyOperations",
            "/informationProtection",
            "/privacy",
            "/security",
        ],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "devices",
        name: "Devices and Intune",
        summary: "Devices, device management, Intune apps, managed devices, and policies.",
        scopes: &[
            "DeviceManagementApps.ReadWrite.All",
            "DeviceManagementManagedDevices.ReadWrite.All",
        ],
        path_prefixes: &[
            "/devices",
            "/devices(deviceId='{deviceId}')",
            "/deviceAppManagement",
            "/deviceManagement",
        ],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "education",
        name: "Education",
        summary: "Classes, schools, education users, assignments, and reports.",
        scopes: &[],
        path_prefixes: &["/education"],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "search",
        name: "Microsoft Search",
        summary: "Search across Microsoft Graph content connectors.",
        scopes: &[
            "ExternalItem.Read.All",
            "Acronym.Read.All",
            "Bookmark.Read.All",
            "QnA.Read.All",
        ],
        path_prefixes: &["/search"],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "external-connections",
        name: "External Connections",
        summary: "External connections, schemas, items, and content connectors.",
        scopes: &[
            "ExternalConnection.ReadWrite.OwnedBy",
            "ExternalItem.ReadWrite.OwnedBy",
        ],
        path_prefixes: &["/connections", "/external"],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "solutions",
        name: "Solutions and Employee Experience",
        summary: "Bookings, virtual events, backup, employee experience, and Copilot.",
        scopes: &[],
        path_prefixes: &["/copilot", "/employeeExperience", "/solutions"],
        exact_paths: &[],
    },
    GraphScopePreset {
        id: "platform-services",
        name: "Platform Services",
        summary: "Places, print, storage, subscriptions, functions, filters, and extensions.",
        scopes: &["Place.Read.All", "Printer.ReadWrite.All"],
        path_prefixes: &[
            "/filterOperators",
            "/functions",
            "/places",
            "/print",
            "/schemaExtensions",
            "/storage",
            "/subscriptions",
        ],
        exact_paths: &[],
    },
];

/// Look up a Google Discovery preset.
#[must_use]
pub fn google_preset(id: &str) -> Option<&'static GooglePreset> {
    GOOGLE_PRESETS.iter().find(|p| p.id == id)
}

/// Look up a Graph scope preset.
#[must_use]
pub fn graph_preset(id: &str) -> Option<&'static GraphScopePreset> {
    GRAPH_SCOPE_PRESETS.iter().find(|p| p.id == id)
}

/// True when `url` is Microsoft's published Graph `OpenAPI` monolith.
#[must_use]
pub fn is_graph_monolith_url(url: &str) -> bool {
    let bare = url.split('#').next().unwrap_or(url).trim();
    bare == MICROSOFT_GRAPH_OPENAPI_URL
        || (bare.contains("microsoftgraph/msgraph-metadata") && bare.contains("/openapi."))
}

/// True when `url` is a Graph HTTP or metadata URL.
#[must_use]
pub fn is_graph_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    is_graph_monolith_url(url)
        || lower.contains("graph.microsoft.com")
        || lower.contains("microsoftgraph")
}

/// Match a Google Discovery URL to a preset.
#[must_use]
pub fn google_preset_for_url(url: &str) -> Option<&'static GooglePreset> {
    let bare = url.split('#').next().unwrap_or(url).trim();
    GOOGLE_PRESETS.iter().find(|p| p.url == bare)
}

/// Keep a Graph path if it matches preset prefixes / exact paths.
#[must_use]
pub fn graph_path_kept(path: &str, prefixes: &[String], exact: &[String]) -> bool {
    if prefixes.is_empty() && exact.is_empty() {
        return true;
    }
    if exact.iter().any(|e| e == path) {
        return true;
    }
    prefixes.iter().any(|prefix| {
        path == prefix || path.starts_with(&format!("{prefix}/")) || path.starts_with(prefix)
    })
}

/// Merge preset fields into an `OpenAPI` integration config.
///
/// # Errors
///
/// Unknown preset id.
pub fn apply_preset(config: &mut Map<String, Value>, preset_id: &str) -> Result<(), PluginError> {
    if let Some(google) = google_preset(preset_id) {
        config
            .entry("specUrl")
            .or_insert_with(|| Value::String(google.url.to_owned()));
        config.insert(
            "authorizationUrl".into(),
            Value::String(GOOGLE_AUTHORIZATION_URL.into()),
        );
        config.insert("tokenUrl".into(), Value::String(GOOGLE_TOKEN_URL.into()));
        config.insert("preset".into(), Value::String(google.id.into()));
        config.insert("presetName".into(), Value::String(google.name.into()));
        return Ok(());
    }
    if let Some(graph) = graph_preset(preset_id) {
        config.insert("preset".into(), Value::String(graph.id.into()));
        config.insert("presetName".into(), Value::String(graph.name.into()));
        config.insert("baseUrl".into(), json!(MICROSOFT_GRAPH_BASE_URL));
        config.insert(
            "authorizationUrl".into(),
            Value::String(MICROSOFT_AUTHORIZATION_URL.into()),
        );
        config.insert("tokenUrl".into(), Value::String(MICROSOFT_TOKEN_URL.into()));
        config.insert(
            "microsoftGraphPathPrefixes".into(),
            json!(graph.path_prefixes),
        );
        config.insert("microsoftGraphExactPaths".into(), json!(graph.exact_paths));
        config.insert("microsoftGraphScopes".into(), json!(graph.scopes));
        return Ok(());
    }
    Err(PluginError::new(format!("unknown preset {preset_id}")))
}

/// Config strings used as Graph path keep-lists.
#[must_use]
pub fn graph_filters(config: &Value) -> (Vec<String>, Vec<String>) {
    let prefixes = string_list(config, "microsoftGraphPathPrefixes");
    let exact = string_list(config, "microsoftGraphExactPaths");
    (prefixes, exact)
}

fn string_list(config: &Value, key: &str) -> Vec<String> {
    config
        .get(key)
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{
        apply_preset, google_preset, graph_path_kept, graph_preset, is_graph_monolith_url,
    };
    use serde_json::Map;

    #[test]
    fn google_gmail_preset_exists() {
        let p = google_preset("google-gmail").expect("gmail");
        assert!(p.url.contains("gmail"));
    }

    #[test]
    fn graph_mail_keeps_messages() {
        let mail = graph_preset("mail").expect("mail");
        let prefixes: Vec<String> = mail.path_prefixes.iter().map(|s| (*s).to_owned()).collect();
        assert!(graph_path_kept("/me/messages", &prefixes, &[]));
        assert!(graph_path_kept("/me/messages/{id}", &prefixes, &[]));
        assert!(!graph_path_kept("/sites", &prefixes, &[]));
    }

    #[test]
    fn refuses_monolith_url() {
        assert!(is_graph_monolith_url(
            "https://raw.githubusercontent.com/microsoftgraph/msgraph-metadata/master/openapi/v1.0/openapi.yaml"
        ));
    }

    #[test]
    fn apply_unknown_preset_errors() {
        let mut map = Map::new();
        assert!(apply_preset(&mut map, "nope").is_err());
    }
}
