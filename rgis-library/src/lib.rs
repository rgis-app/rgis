use std::sync;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    GeoJson,
    GeoTiff,
}

pub struct Entry {
    pub name: &'static str,
    pub url: &'static str,
    pub crs: u16,
}

impl Entry {
    /// The file format, determined by the URL's extension.
    pub fn format(&self) -> Option<Format> {
        if self.url.ends_with(".geojson") {
            Some(Format::GeoJson)
        } else if self.url.ends_with(".tif") || self.url.ends_with(".tiff") {
            Some(Format::GeoTiff)
        } else {
            None
        }
    }
}

pub struct Folder {
    pub name: &'static str,
    pub entries: Vec<Entry>,
    pub sub_folders: Vec<Folder>,
}

fn build() -> Vec<Folder> {
    vec![
        Folder {
            name: "World",
            entries: vec![
                Entry {
                    name: "Countries",
                    url: "https://storage.googleapis.com/rgis-library/World/Countries.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Land",
                    url: "https://storage.googleapis.com/rgis-library/World/Land.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Ocean",
                    url: "https://storage.googleapis.com/rgis-library/World/Ocean.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Coastline",
                    url: "https://storage.googleapis.com/rgis-library/World/Coastline.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Boundary Lines",
                    url: "https://storage.googleapis.com/rgis-library/World/Boundary-Lines.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Disputed Areas",
                    url: "https://storage.googleapis.com/rgis-library/World/Disputed-Areas.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Lakes",
                    url: "https://storage.googleapis.com/rgis-library/World/Lakes.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Lakes (Europe)",
                    url: "https://storage.googleapis.com/rgis-library/World/Lakes-Europe.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Rivers",
                    url: "https://storage.googleapis.com/rgis-library/World/Rivers.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Rivers (Europe)",
                    url: "https://storage.googleapis.com/rgis-library/World/Rivers-Europe.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Glaciated Areas",
                    url: "https://storage.googleapis.com/rgis-library/World/Glaciated-Areas.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Antarctic Ice Shelves",
                    url: "https://storage.googleapis.com/rgis-library/World/Antarctic-Ice-Shelves.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Coral Reefs",
                    url: "https://storage.googleapis.com/rgis-library/World/Coral-Reefs.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Playas",
                    url: "https://storage.googleapis.com/rgis-library/World/Playas.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Tectonic Plate Boundaries",
                    url: "https://storage.googleapis.com/rgis-library/World/Tectonic-Plate-Boundaries.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Airports",
                    url: "https://storage.googleapis.com/rgis-library/World/Airports.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Ports",
                    url: "https://storage.googleapis.com/rgis-library/World/Ports.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Shipping Lanes",
                    url: "https://storage.googleapis.com/rgis-library/World/Shipping-Lanes.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Populated Places",
                    url: "https://storage.googleapis.com/rgis-library/World/Populated-Places.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Urban Areas",
                    url: "https://storage.googleapis.com/rgis-library/World/Urban-Areas.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Elevation Points",
                    url: "https://storage.googleapis.com/rgis-library/World/Elevation-Points.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Geographic Regions",
                    url: "https://storage.googleapis.com/rgis-library/World/Geographic-Regions.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Marine Regions",
                    url: "https://storage.googleapis.com/rgis-library/World/Marine-Regions.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Time Zones",
                    url: "https://storage.googleapis.com/rgis-library/World/Time-Zones.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Geographic Lines",
                    url: "https://storage.googleapis.com/rgis-library/World/Geographic-Lines.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Graticules",
                    url: "https://storage.googleapis.com/rgis-library/World/Graticules.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Shaded Relief",
                    url: "https://storage.googleapis.com/rgis-library/World/Shaded-Relief.tif",
                    crs: 4326,
                },
                Entry {
                    name: "Natural Earth Shaded Relief",
                    url: "https://storage.googleapis.com/rgis-library/World/Natural-Earth-Shaded-Relief.tif",
                    crs: 4326,
                },
            ],
            sub_folders: vec![],
        },
        Folder {
            name: "Argentina",
            entries: vec![],
            sub_folders: vec![Folder {
                name: "Buenos Aires",
                entries: vec![Entry {
                    name: "Neighbourhoods",
                    url: "https://storage.googleapis.com/rgis-library/Cities/Buenos-Aires-Neighbourhoods.geojson",
                    crs: 4326,
                }],
                sub_folders: vec![],
            }],
        },
        Folder {
            name: "Australia",
            entries: vec![Entry {
                name: "States",
                url: "https://storage.googleapis.com/rgis-library/Countries/Australia-States.geojson",
                crs: 4326,
            }],
            sub_folders: vec![Folder {
                name: "Sydney",
                entries: vec![Entry {
                    name: "Suburbs",
                    url: "https://storage.googleapis.com/rgis-library/Cities/Sydney-Suburbs.geojson",
                    crs: 4326,
                }],
                sub_folders: vec![],
            }],
        },
        Folder {
            name: "Brazil",
            entries: vec![Entry {
                name: "States",
                url: "https://storage.googleapis.com/rgis-library/Countries/Brazil-States.geojson",
                crs: 4326,
            }],
            sub_folders: vec![],
        },
        Folder {
            name: "Canada",
            entries: vec![],
            sub_folders: vec![Folder {
                name: "Toronto",
                entries: vec![Entry {
                    name: "Neighbourhoods",
                    url: "https://storage.googleapis.com/rgis-library/Cities/Toronto-Neighbourhoods.geojson",
                    crs: 4326,
                }],
                sub_folders: vec![],
            }],
        },
        Folder {
            name: "Caribbean",
            entries: vec![Entry {
                name: "Islands",
                url: "https://storage.googleapis.com/rgis-library/Countries/Caribbean-Islands.geojson",
                crs: 4326,
            }],
            sub_folders: vec![],
        },
        Folder {
            name: "Denmark",
            entries: vec![],
            sub_folders: vec![Folder {
                name: "Copenhagen",
                entries: vec![Entry {
                    name: "Districts",
                    url: "https://storage.googleapis.com/rgis-library/Cities/Copenhagen-Districts.geojson",
                    crs: 4326,
                }],
                sub_folders: vec![],
            }],
        },
        Folder {
            name: "France",
            entries: vec![Entry {
                name: "Regions",
                url: "https://storage.googleapis.com/rgis-library/Countries/France-Regions.geojson",
                crs: 4326,
            }],
            sub_folders: vec![Folder {
                name: "Paris",
                entries: vec![Entry {
                    name: "Arrondissements",
                    url: "https://storage.googleapis.com/rgis-library/Cities/Paris-Arrondissements.geojson",
                    crs: 4326,
                }],
                sub_folders: vec![],
            }],
        },
        Folder {
            name: "Germany",
            entries: vec![],
            sub_folders: vec![Folder {
                name: "Berlin",
                entries: vec![Entry {
                    name: "Districts",
                    url: "https://storage.googleapis.com/rgis-library/Cities/Berlin-Districts.geojson",
                    crs: 4326,
                }],
                sub_folders: vec![],
            }],
        },
        Folder {
            name: "India",
            entries: vec![Entry {
                name: "States",
                url: "https://storage.googleapis.com/rgis-library/Countries/India-States.geojson",
                crs: 4326,
            }],
            sub_folders: vec![],
        },
        Folder {
            name: "Ireland",
            entries: vec![Entry {
                name: "Counties",
                url: "https://storage.googleapis.com/rgis-library/Countries/Ireland-Counties.geojson",
                crs: 4326,
            }],
            sub_folders: vec![],
        },
        Folder {
            name: "Italy",
            entries: vec![],
            sub_folders: vec![
                Folder {
                    name: "Rome",
                    entries: vec![Entry {
                        name: "Rioni",
                        url: "https://storage.googleapis.com/rgis-library/Cities/Rome-Rioni.geojson",
                        crs: 4326,
                    }],
                    sub_folders: vec![],
                },
                Folder {
                    name: "Venice",
                    entries: vec![Entry {
                        name: "Islands",
                        url: "https://storage.googleapis.com/rgis-library/Cities/Venice.geojson",
                        crs: 4326,
                    }],
                    sub_folders: vec![],
                },
            ],
        },
        Folder {
            name: "Japan",
            entries: vec![Entry {
                name: "Prefectures",
                url: "https://storage.googleapis.com/rgis-library/Countries/Japan-Prefectures.geojson",
                crs: 4326,
            }],
            sub_folders: vec![],
        },
        Folder {
            name: "Mexico",
            entries: vec![Entry {
                name: "States",
                url: "https://storage.googleapis.com/rgis-library/Countries/Mexico-States.geojson",
                crs: 4326,
            }],
            sub_folders: vec![],
        },
        Folder {
            name: "Netherlands",
            entries: vec![],
            sub_folders: vec![Folder {
                name: "Amsterdam",
                entries: vec![Entry {
                    name: "Neighbourhoods",
                    url: "https://storage.googleapis.com/rgis-library/Cities/Amsterdam-Neighbourhoods.geojson",
                    crs: 4326,
                }],
                sub_folders: vec![],
            }],
        },
        Folder {
            name: "Russia",
            entries: vec![Entry {
                name: "Country",
                url: "https://storage.googleapis.com/rgis-library/Russia.geojson",
                crs: 4326,
            }],
            sub_folders: vec![Folder {
                name: "Moscow",
                entries: vec![Entry {
                    name: "Districts",
                    url: "https://storage.googleapis.com/rgis-library/Cities/Moscow-Districts.geojson",
                    crs: 4326,
                }],
                sub_folders: vec![],
            }],
        },
        Folder {
            name: "South Korea",
            entries: vec![],
            sub_folders: vec![Folder {
                name: "Seoul",
                entries: vec![Entry {
                    name: "Districts",
                    url: "https://storage.googleapis.com/rgis-library/Cities/Seoul-Districts.geojson",
                    crs: 4326,
                }],
                sub_folders: vec![],
            }],
        },
        Folder {
            name: "Spain",
            entries: vec![Entry {
                name: "Provinces",
                url: "https://storage.googleapis.com/rgis-library/Countries/Spain-Provinces.geojson",
                crs: 4326,
            }],
            sub_folders: vec![],
        },
        Folder {
            name: "UK",
            entries: vec![],
            sub_folders: vec![Folder {
                name: "London",
                entries: vec![
                    Entry {
                        name: "Boroughs",
                        url: "https://storage.googleapis.com/rgis-library/Cities/London-Boroughs.geojson",
                        crs: 4326,
                    },
                    Entry {
                        name: "Underground Lines",
                        url: "https://storage.googleapis.com/rgis-library/Cities/London-Underground.geojson",
                        crs: 4326,
                    },
                ],
                sub_folders: vec![],
            }],
        },
        Folder {
            name: "USA",
            entries: vec![
                Entry {
                    name: "States",
                    url: "https://storage.googleapis.com/rgis-library/USA/States.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "National Parks",
                    url: "https://storage.googleapis.com/rgis-library/USA/Parks.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Railroads",
                    url: "https://storage.googleapis.com/rgis-library/USA/Railroads.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Amtrak Stations",
                    url: "https://storage.googleapis.com/rgis-library/USA/Amtrak-Stations.geojson",
                    crs: 4326,
                },
                Entry {
                    name: "Amtrak Tracks",
                    url: "https://storage.googleapis.com/rgis-library/USA/Amtrak-Tracks.geojson",
                    crs: 4326,
                },
            ],
            sub_folders: vec![
                Folder {
                    name: "California",
                    entries: vec![Entry {
                        name: "Counties",
                        url: "https://storage.googleapis.com/rgis-library/Countries/USA-California-Counties.geojson",
                        crs: 4326,
                    }],
                    sub_folders: vec![],
                },
                Folder {
                    name: "New York City",
                    entries: vec![
                        Entry {
                            name: "Subway Lines",
                            url: "https://storage.googleapis.com/rgis-library/NYC/Subway-Lines.geojson",
                            crs: 4326,
                        },
                        Entry {
                            name: "Subway Stations",
                            url: "https://storage.googleapis.com/rgis-library/NYC/Subway-Stations.geojson",
                            crs: 4326,
                        },
                        Entry {
                            name: "Bike Routes",
                            url: "https://storage.googleapis.com/rgis-library/NYC/Bike-Routes.geojson",
                            crs: 4326,
                        },
                        Entry {
                            name: "Taxi Zones",
                            url: "https://storage.googleapis.com/rgis-library/NYC/Taxi-Zones.geojson",
                            crs: 4326,
                        },
                        Entry {
                            name: "Police Precincts",
                            url: "https://storage.googleapis.com/rgis-library/NYC/Police-Precincts.geojson",
                            crs: 4326,
                        },
                        Entry {
                            name: "WiFi Hotspots",
                            url: "https://storage.googleapis.com/rgis-library/NYC/WiFi-Hotspots.geojson",
                            crs: 4326,
                        },
                    ],
                    sub_folders: vec![],
                },
                Folder {
                    name: "Washington D.C.",
                    entries: vec![Entry {
                        name: "Street Centerlines",
                        url: "https://storage.googleapis.com/rgis-library/USA/DC-Street-Centerlines.geojson",
                        crs: 4326,
                    }],
                    sub_folders: vec![],
                },
            ],
        },
    ]
}

pub fn get() -> &'static Vec<Folder> {
    static LIBRARY: sync::OnceLock<Vec<Folder>> = sync::OnceLock::new();
    LIBRARY.get_or_init(build)
}

#[cfg(test)]
mod tests {
    fn all_entries<'a>(folders: &'a [super::Folder], out: &mut Vec<&'a super::Entry>) {
        for folder in folders {
            out.extend(&folder.entries);
            all_entries(&folder.sub_folders, out);
        }
    }

    #[test]
    fn every_entry_has_a_known_format() {
        let mut entries = vec![];
        all_entries(super::get(), &mut entries);
        for entry in entries {
            assert!(entry.format().is_some(), "unknown format: {}", entry.url);
        }
    }
}
